//! `functor.depth-frame/1`: one environment depth frame as `Xr.streamDepth`
//! POSTs it (`docs/vr.md`, "Depth streaming").
//!
//! The body is a big-endian u32 header length, the UTF-8 JSON header, then
//! `payload_len` bytes of RGBA8 whose RGB carry 24-bit window depth, most
//! significant byte first, rows bottom-up — the readback's bytes as-is.

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "functor.depth-frame/1";
pub const CONTENT_TYPE: &str = "application/vnd.functor.depth-frame";
pub const ENCODING: &str = "rgba8-depth24";
pub const ROWS: &str = "bottom-up";

/// Where a frame's depth came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthFrameSource {
    /// The headset's depth sensor.
    Device,
    /// The desktop stand-in room's rendered depth.
    Emulated,
}

/// When a frame was sensed, in the named clock's nanoseconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthFrameTime {
    /// `"xr"`: OpenXR time. `"steady"`: the sender's monotonic clock.
    pub clock: String,
    pub display_ns: i64,
    pub acquire_ns: Option<i64>,
    pub sensor_ns: Option<i64>,
}

/// The JSON header. Matrices are column-major.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthFrameHeader {
    pub schema: String,
    /// A new id each time a stream starts.
    pub stream: String,
    /// Per stream, +1 per frame queued; a gap is a dropped frame.
    pub seq: u64,
    pub source: DepthFrameSource,
    pub eye: String,
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    pub rows: String,
    pub camera_to_world: [f32; 16],
    pub projection: [f32; 16],
    /// `projection · inverse(camera_to_world)`.
    pub view_projection: [f32; 16],
    pub near: f32,
    /// `None` (JSON `null`): an infinite far plane.
    pub far: Option<f32>,
    pub tracking: bool,
    /// +1 whenever the world frame changes (a recentre).
    pub space_epoch: u64,
    pub time: DepthFrameTime,
    pub payload_len: u64,
}

/// The request body: header length, header, payload.
pub fn encode(header: &DepthFrameHeader, payload: &[u8]) -> Vec<u8> {
    debug_assert_eq!(header.payload_len, payload.len() as u64);
    let json = serde_json::to_vec(header).expect("a depth frame header serializes");
    let mut body = Vec::with_capacity(4 + json.len() + payload.len());
    body.extend_from_slice(&(json.len() as u32).to_be_bytes());
    body.extend_from_slice(&json);
    body.extend_from_slice(payload);
    body
}

/// Split a request body into its header and payload, rejecting any other
/// schema, encoding or row order, and any length that does not add up.
pub fn decode(body: &[u8]) -> Result<(DepthFrameHeader, &[u8]), String> {
    let len_bytes: [u8; 4] = body
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or("depth frame: shorter than its 4-byte header length")?;
    let header_len = u32::from_be_bytes(len_bytes) as usize;
    let json = body
        .get(4..4 + header_len)
        .ok_or_else(|| format!("depth frame: header length {header_len} overruns the body"))?;
    let header: DepthFrameHeader =
        serde_json::from_slice(json).map_err(|e| format!("depth frame: bad header: {e}"))?;
    if header.schema != SCHEMA {
        return Err(format!("depth frame: schema {:?}, expected {SCHEMA:?}", header.schema));
    }
    if header.encoding != ENCODING || header.rows != ROWS {
        return Err(format!(
            "depth frame: encoding {:?} rows {:?}, expected {ENCODING:?} {ROWS:?}",
            header.encoding, header.rows
        ));
    }
    let payload = &body[4 + header_len..];
    let expected = header.width as u64 * header.height as u64 * 4;
    if header.payload_len != expected || payload.len() as u64 != expected {
        return Err(format!(
            "depth frame: {}x{} RGBA8 is {expected} bytes; payload_len {}, body has {}",
            header.width,
            header.height,
            header.payload_len,
            payload.len()
        ));
    }
    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32) -> DepthFrameHeader {
        let identity: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
        DepthFrameHeader {
            schema: SCHEMA.to_string(),
            stream: "a1b2c3".to_string(),
            seq: 7,
            source: DepthFrameSource::Emulated,
            eye: "left".to_string(),
            width,
            height,
            encoding: ENCODING.to_string(),
            rows: ROWS.to_string(),
            camera_to_world: identity,
            projection: identity,
            view_projection: identity,
            near: 0.1,
            far: None,
            tracking: true,
            space_epoch: 0,
            time: DepthFrameTime {
                clock: "steady".to_string(),
                display_ns: 123,
                acquire_ns: None,
                sensor_ns: None,
            },
            payload_len: width as u64 * height as u64 * 4,
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let header = header(2, 3);
        let payload: Vec<u8> = (0..24).collect();
        let body = encode(&header, &payload);
        let (decoded, bytes) = decode(&body).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(bytes, payload);
    }

    #[test]
    fn the_header_matches_the_documented_shape() {
        let json = serde_json::to_value(header(320, 320)).unwrap();
        assert_eq!(json["schema"], "functor.depth-frame/1");
        assert_eq!(json["source"], "emulated");
        assert_eq!(json["far"], serde_json::Value::Null);
        assert_eq!(json["time"]["acquire_ns"], serde_json::Value::Null);
        assert_eq!(json["camera_to_world"].as_array().unwrap().len(), 16);
        assert_eq!(json["payload_len"], 409_600);
    }

    #[test]
    fn malformed_frames_are_rejected() {
        let good = encode(&header(2, 2), &[0; 16]);
        // Truncated length, overrunning length, garbage header.
        assert!(decode(&good[..3]).is_err());
        let mut overrun = good.clone();
        overrun[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&overrun).is_err());
        let mut garbage = 2u32.to_be_bytes().to_vec();
        garbage.extend_from_slice(b"{]");
        assert!(decode(&garbage).is_err());
        // A payload shorter or longer than width x height x 4.
        assert!(decode(&good[..good.len() - 1]).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        // payload_len disagreeing with the dimensions.
        let mut lying = header(2, 2);
        lying.payload_len = 12;
        assert!(decode(&encode_unchecked(&lying, &[0; 12])).is_err());
        // Another schema or encoding.
        for edit in [
            (|h: &mut DepthFrameHeader| h.schema = "functor.depth-frame/2".into())
                as fn(&mut DepthFrameHeader),
            |h| h.encoding = "r32f".into(),
            |h| h.rows = "top-down".into(),
        ] {
            let mut h = header(2, 2);
            edit(&mut h);
            assert!(decode(&encode(&h, &[0; 16])).is_err());
        }
    }

    /// `encode` without its length assertion, to build a lying body.
    fn encode_unchecked(header: &DepthFrameHeader, payload: &[u8]) -> Vec<u8> {
        let json = serde_json::to_vec(header).unwrap();
        let mut body = (json.len() as u32).to_be_bytes().to_vec();
        body.extend_from_slice(&json);
        body.extend_from_slice(payload);
        body
    }
}
