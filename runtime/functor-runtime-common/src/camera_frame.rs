//! `functor.camera-frame/1`: one passthrough camera image as
//! `Xr.streamCamera` POSTs it (`docs/vr.md`, "Camera streaming").
//!
//! The body is framed like a depth frame: a big-endian u32 header length,
//! the UTF-8 JSON header, then `payload_len` bytes of I420 — the Y plane,
//! then U, then V at half resolution, rows top-down.

use cgmath::Matrix4;
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "functor.camera-frame/1";
pub const CONTENT_TYPE: &str = "application/vnd.functor.camera-frame";
pub const ENCODING: &str = "i420";
/// The widest or tallest image `decode` accepts.
const MAX_SIDE: u32 = 16384;

/// Where a frame's image came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraFrameSource {
    /// The headset's passthrough camera.
    Device,
    /// The desktop stand-in room, rendered.
    Emulated,
}

/// A pinhole camera in pixels of the delivered image, OpenCV conventions:
/// `u = fx·x/z + cx`, `v = fy·y/z + cy`, pixel centres at integers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intrinsics {
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
}

impl Intrinsics {
    /// The pixel `point` (camera frame: x right, y down, z forward) lands
    /// on; `None` behind the camera.
    pub fn project(&self, point: [f32; 3]) -> Option<[f32; 2]> {
        let [x, y, z] = point;
        (z > 0.0).then(|| [self.fx * x / z + self.cx, self.fy * y / z + self.cy])
    }

    /// The GL projection whose window pixels are this camera's, for a
    /// `width`×`height` image read back bottom-up and flipped.
    pub fn gl_projection(&self, width: u32, height: u32, near: f32, far: f32) -> Matrix4<f32> {
        // The view angle to an image edge, `px` pixels in; centres sit half
        // a pixel inside the edges.
        let edge = |px: u32, c: f32, f: f32| ((px as f32 - c - 0.5) / f).atan();
        let mut camera = crate::Camera::default();
        camera.near = near;
        camera.far = far;
        // Image rows run down; GL's y runs up.
        camera.projection_matrix_from_fov_angles(
            edge(0, self.cx, self.fx),
            edge(width, self.cx, self.fx),
            -edge(height, self.cy, self.fy),
            -edge(0, self.cy, self.fy),
        )
    }
}

/// When a frame was exposed, in the named clock's nanoseconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraFrameTime {
    /// `"xr"`: OpenXR time, as depth frames' `display_ns`. `"steady"`: the
    /// sender's monotonic clock.
    pub clock: String,
    /// The start of exposure (first row); the pose is the head's then.
    pub exposure_ns: i64,
    /// The camera's own timestamp (Android `CLOCK_MONOTONIC`); `None` when
    /// emulated.
    pub sensor_ns: Option<i64>,
}

/// The JSON header. Matrices are column-major; the camera frame is
/// OpenCV's: x right, y down, z forward.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraFrameHeader {
    pub schema: String,
    /// A new id each time a stream starts.
    pub stream: String,
    /// Per stream, +1 per frame queued; a gap is a dropped frame.
    pub seq: u64,
    pub source: CameraFrameSource,
    /// `"left"` or `"right"`, as the wearer sees them.
    pub camera: String,
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    pub intrinsics: Intrinsics,
    /// Android `LENS_DISTORTION` `[k1, k2, k3, p1, p2]`; `None` (JSON
    /// `null`): the image is pinhole.
    pub distortion: Option<[f32; 5]>,
    /// Camera → world, the world depth frames use.
    pub camera_to_world: [f32; 16],
    /// Camera → head (OpenXR VIEW space: x right, y up, z back).
    pub camera_to_head: [f32; 16],
    pub time: CameraFrameTime,
    /// `false`: the head pose was not valid at exposure; `camera_to_world`
    /// is then the identity.
    pub tracking: bool,
    /// +1 whenever the world frame changes (a recentre).
    pub space_epoch: u64,
    pub payload_len: u64,
}

/// An I420 image's byte length: a full Y plane and half-resolution U and V.
pub fn i420_len(width: u32, height: u32) -> u64 {
    let (w, h) = (width as u64, height as u64);
    w * h + 2 * w.div_ceil(2) * h.div_ceil(2)
}

/// The request body: header length, header, payload.
pub fn encode(header: &CameraFrameHeader, payload: &[u8]) -> Vec<u8> {
    debug_assert_eq!(header.payload_len, payload.len() as u64);
    crate::depth_frame::join(header, payload)
}

/// Split a request body into its header and payload, rejecting any other
/// schema or encoding, and any length that does not add up.
pub fn decode(body: &[u8]) -> Result<(CameraFrameHeader, &[u8]), String> {
    let (json, payload) = crate::depth_frame::split(body, "camera frame")?;
    let header: CameraFrameHeader =
        serde_json::from_slice(json).map_err(|e| format!("camera frame: bad header: {e}"))?;
    if header.schema != SCHEMA {
        return Err(format!(
            "camera frame: schema {:?}, expected {SCHEMA:?}",
            header.schema
        ));
    }
    if header.encoding != ENCODING {
        return Err(format!(
            "camera frame: encoding {:?}, expected {ENCODING:?}",
            header.encoding
        ));
    }
    if header.width > MAX_SIDE || header.height > MAX_SIDE {
        return Err(format!(
            "camera frame: {}x{} is larger than {MAX_SIDE} a side",
            header.width, header.height
        ));
    }
    let expected = i420_len(header.width, header.height);
    if header.payload_len != expected || payload.len() as u64 != expected {
        return Err(format!(
            "camera frame: {}x{} I420 is {expected} bytes; payload_len {}, body has {}",
            header.width,
            header.height,
            header.payload_len,
            payload.len()
        ));
    }
    Ok((header, payload))
}

/// RGBA8 rows (bottom-up when `bottom_up`, as GL reads them) → I420, rows
/// top-down. BT.601 full range (JFIF), as Android cameras deliver; each
/// chroma sample averages its 2×2 block.
pub fn rgba_to_i420(rgba: &[u8], width: u32, height: u32, bottom_up: bool) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; i420_len(width, height) as usize];
    let (y_plane, chroma) = out.split_at_mut(w * h);
    let (u_plane, v_plane) = chroma.split_at_mut(cw * ch);
    let pixel = |x: usize, y: usize| {
        let row = if bottom_up { h - 1 - y } else { y };
        let p = &rgba[(row * w + x) * 4..][..3];
        [p[0] as f32, p[1] as f32, p[2] as f32]
    };
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = pixel(x, y);
            y_plane[y * w + x] = byte(0.299 * r + 0.587 * g + 0.114 * b);
        }
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut sum, mut n) = ([0.0f32; 3], 0.0);
            for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (cx * 2 + x, cy * 2 + y);
                if x < w && y < h {
                    let p = pixel(x, y);
                    (0..3).for_each(|c| sum[c] += p[c]);
                    n += 1.0;
                }
            }
            let [r, g, b] = sum.map(|c| c / n);
            u_plane[cy * cw + cx] = byte(-0.168736 * r - 0.331264 * g + 0.5 * b + 128.0);
            v_plane[cy * cw + cx] = byte(0.5 * r - 0.418688 * g - 0.081312 * b + 128.0);
        }
    }
    out
}

/// One plane of an Android `YUV_420_888` image: its bytes and strides.
pub struct YuvPlane<'a> {
    pub data: &'a [u8],
    pub row_stride: usize,
    pub pixel_stride: usize,
}

/// A `YUV_420_888` image (planes Y, U, V, any strides — planar or
/// semi-planar) → I420.
pub fn yuv420_888_to_i420(width: u32, height: u32, planes: [YuvPlane; 3]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = Vec::with_capacity(i420_len(width, height) as usize);
    for (plane, (pw, ph)) in planes.iter().zip([(w, h), (cw, ch), (cw, ch)]) {
        for y in 0..ph {
            let row = &plane.data[y * plane.row_stride..];
            match plane.pixel_stride {
                1 => out.extend_from_slice(&row[..pw]),
                stride => out.extend((0..pw).map(|x| row[x * stride])),
            }
        }
    }
    out
}

/// `[fx, fy, cx, cy]` calibrated on a `sensor` (width, height) array, for an
/// `output` image the camera crops from its centre to the output's aspect
/// and scales. Pixel centres at integers on both sides.
pub fn rescale_intrinsics(
    calibration: [f32; 4],
    sensor: (u32, u32),
    output: (u32, u32),
) -> Intrinsics {
    let [fx, fy, cx, cy] = calibration;
    let (sw, sh) = (sensor.0 as f32, sensor.1 as f32);
    let (ow, oh) = (output.0 as f32, output.1 as f32);
    let crop_w = sw.min(sh * ow / oh);
    let crop_h = crop_w * oh / ow;
    let (x0, y0) = ((sw - crop_w) / 2.0, (sh - crop_h) / 2.0);
    let k = ow / crop_w;
    Intrinsics {
        fx: fx * k,
        fy: fy * k,
        cx: (cx + 0.5 - x0) * k - 0.5,
        cy: (cy + 0.5 - y0) * k - 0.5,
    }
}

/// Android's lens pose — `p_camera = R(q) · p_head`, `q` as `[x, y, z, w]`,
/// the optical centre at `t` in head axes — as OpenCV camera → head.
pub fn lens_to_head(q: [f32; 4], t: [f32; 3]) -> Matrix4<f32> {
    let rotation = cgmath::Quaternion::new(q[3], q[0], q[1], q[2]);
    Matrix4::from_translation(t.into()) * Matrix4::from(rotation.conjugate())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::Vector4;

    fn header(width: u32, height: u32) -> CameraFrameHeader {
        let identity: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
        CameraFrameHeader {
            schema: SCHEMA.to_string(),
            stream: "a1b2c3".to_string(),
            seq: 7,
            source: CameraFrameSource::Emulated,
            camera: "left".to_string(),
            width,
            height,
            encoding: ENCODING.to_string(),
            intrinsics: Intrinsics {
                fx: 500.0,
                fy: 500.0,
                cx: 320.0,
                cy: 240.0,
            },
            distortion: None,
            camera_to_world: identity,
            camera_to_head: identity,
            time: CameraFrameTime {
                clock: "steady".to_string(),
                exposure_ns: 123,
                sensor_ns: None,
            },
            tracking: true,
            space_epoch: 0,
            payload_len: i420_len(width, height),
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let header = header(4, 2);
        let payload: Vec<u8> = (0..12).collect();
        let body = encode(&header, &payload);
        let (decoded, bytes) = decode(&body).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(bytes, payload);
    }

    #[test]
    fn the_header_matches_the_documented_shape() {
        let json = serde_json::to_value(header(640, 480)).unwrap();
        assert_eq!(json["schema"], "functor.camera-frame/1");
        assert_eq!(json["source"], "emulated");
        assert_eq!(json["camera"], "left");
        assert_eq!(json["encoding"], "i420");
        assert_eq!(json["intrinsics"]["cx"], 320.0);
        assert_eq!(json["distortion"], serde_json::Value::Null);
        assert_eq!(json["time"]["sensor_ns"], serde_json::Value::Null);
        assert_eq!(json["camera_to_head"].as_array().unwrap().len(), 16);
        assert_eq!(json["payload_len"], 460_800);
    }

    #[test]
    fn malformed_frames_are_rejected() {
        let good = encode(&header(2, 2), &[0; 6]);
        assert!(decode(&good[..3]).is_err());
        assert!(decode(&good[..good.len() - 1]).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        for edit in [
            (|h: &mut CameraFrameHeader| h.schema = "functor.camera-frame/2".into())
                as fn(&mut CameraFrameHeader),
            |h| h.encoding = "nv12".into(),
            |h| h.payload_len = 4,
            |h| (h.width, h.height) = (u32::MAX, u32::MAX),
        ] {
            let mut h = header(2, 2);
            edit(&mut h);
            assert!(decode(&crate::depth_frame::join(&h, &[0; 6])).is_err());
        }
    }

    #[test]
    fn the_gl_projection_lands_on_the_intrinsics_pixel() {
        let k = Intrinsics {
            fx: 480.0,
            fy: 520.0,
            cx: 300.0,
            cy: 260.0,
        };
        let (w, h) = (640u32, 480u32);
        let projection = k.gl_projection(w, h, 0.1, 50.0);
        // A point in the OpenCV camera frame, and the same in GL's (y up, z back).
        let cv = [0.4f32, -0.3, 2.0];
        let clip = projection * Vector4::new(cv[0], -cv[1], -cv[2], 1.0);
        let (ndc_x, ndc_y) = (clip.x / clip.w, clip.y / clip.w);
        // GL window pixel centres, then rows flipped top-down.
        let column = (ndc_x + 1.0) / 2.0 * w as f32 - 0.5;
        let row = h as f32 - 1.0 - ((ndc_y + 1.0) / 2.0 * h as f32 - 0.5);
        let [u, v] = k.project(cv).unwrap();
        assert!(
            (column - u).abs() < 1e-3 && (row - v).abs() < 1e-3,
            "{column},{row} vs {u},{v}"
        );
        assert_eq!(k.project([0.0, 0.0, -1.0]), None);
    }

    #[test]
    fn rgba_converts_to_i420_top_down() {
        // 2x2, bottom-up: the bottom row red, the top row white.
        let rgba = [
            255, 0, 0, 255, 255, 0, 0, 255, //
            255, 255, 255, 255, 255, 255, 255, 255,
        ];
        let yuv = rgba_to_i420(&rgba, 2, 2, true);
        assert_eq!(yuv.len(), 6);
        assert_eq!(&yuv[..4], &[255, 255, 76, 76]);
        // The block averages white and red: U below 128, V above.
        assert!(yuv[4] < 128 && yuv[5] > 128, "{yuv:?}");
        // Odd sizes round the chroma planes up.
        assert_eq!(rgba_to_i420(&[0; 3 * 3 * 4], 3, 3, false).len(), 9 + 2 * 4);
    }

    #[test]
    fn semi_planar_yuv_repacks_as_i420() {
        // 4x2 with padded rows: Y, then interleaved chroma as NV12 delivers
        // it (U plane = UVUV…, V plane = the same bytes shifted by one).
        let y = [1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0];
        let uv = [10, 20, 11, 21];
        let yuv = yuv420_888_to_i420(
            4,
            2,
            [
                YuvPlane {
                    data: &y,
                    row_stride: 6,
                    pixel_stride: 1,
                },
                YuvPlane {
                    data: &uv,
                    row_stride: 4,
                    pixel_stride: 2,
                },
                YuvPlane {
                    data: &uv[1..],
                    row_stride: 4,
                    pixel_stride: 2,
                },
            ],
        );
        assert_eq!(yuv, [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 20, 21]);
    }

    #[test]
    fn intrinsics_follow_the_centre_crop_and_scale() {
        // A square 1728 sensor streamed at 1280x960: the centre 1728x1296,
        // scaled by 1280/1728.
        let k = rescale_intrinsics([1172.5, 1172.5, 863.5, 863.5], (1728, 1728), (1280, 960));
        let scale = 1280.0 / 1728.0;
        assert!((k.fx - 1172.5 * scale).abs() < 1e-3);
        // The sensor's centre stays the image's centre.
        assert!(
            (k.cx - 639.5).abs() < 1e-3 && (k.cy - 479.5).abs() < 1e-3,
            "{k:?}"
        );
        // The same size is the identity.
        let same = rescale_intrinsics([500.0, 501.0, 300.0, 200.0], (640, 480), (640, 480));
        assert_eq!(
            same,
            Intrinsics {
                fx: 500.0,
                fy: 501.0,
                cx: 300.0,
                cy: 200.0
            }
        );
    }

    #[test]
    fn a_lens_pose_places_the_camera_on_the_head() {
        // A Quest 3 left camera: about 169° about x (y down, z forward,
        // pitched down), 3 cm left, 2 cm down, 6 cm forward of the head.
        let t = [-0.0323, -0.0175, -0.0632];
        let to_head = lens_to_head([-0.9953, 0.0027, -0.0041, 0.0970], t);
        let forward = to_head * Vector4::new(0.0, 0.0, 1.0, 0.0);
        // OpenXR VIEW looks down -z: the camera looks forward and a little down.
        assert!(forward.z < -0.97 && forward.y < -0.1 && forward.y > -0.25, "{forward:?}");
        let down = to_head * Vector4::new(0.0, 1.0, 0.0, 0.0);
        assert!(down.y < -0.97, "{down:?}");
        assert_eq!(to_head.w.truncate(), t.into());
    }
}
