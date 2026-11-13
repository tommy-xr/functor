//! `GET /stream`: live JPEG frames of the rendered framebuffer.
//!
//! The render loop's only work is a framebuffer readback, and only on a frame
//! after a client asked for one ([`FrameTap::take_request`]): no client, no
//! readback. Flipping, downscaling and encoding run on the client's own
//! connection thread. The tap holds just the latest frame, so a slow client
//! skips frames instead of queueing them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The multipart boundary between parts.
pub const BOUNDARY: &str = "functor-frame";
/// JPEG quality when `q` is absent: below 90 the encoder subsamples chroma
/// 4:2:0, about twice as fast as 4:4:4.
pub const DEFAULT_QUALITY: u8 = 80;

/// `GET /stream`'s query parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamParams {
    /// Frames are scaled down to fit within `w` × `h` (either may be absent),
    /// never up.
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub quality: u8,
    /// At most this many frames per second; `None` = one per rendered frame
    /// the client keeps up with.
    pub fps: Option<f32>,
    /// `application/octet-stream` instead of `multipart/x-mixed-replace`.
    pub octet: bool,
}

impl Default for StreamParams {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            quality: DEFAULT_QUALITY,
            fps: None,
            octet: false,
        }
    }
}

impl StreamParams {
    /// The response's `Content-Type`; the body bytes are the same either way.
    pub fn content_type(&self) -> String {
        if self.octet {
            "application/octet-stream".to_string()
        } else {
            format!("multipart/x-mixed-replace; boundary={BOUNDARY}")
        }
    }

    /// Minimum time between parts.
    pub fn interval(&self) -> Duration {
        self.fps
            .map_or(Duration::ZERO, |fps| Duration::from_secs_f32(1.0 / fps))
    }
}

/// Parse the query (the part after `?`); a malformed value is an error.
pub fn parse_query(query: &str) -> Result<StreamParams, String> {
    fn positive(key: &str, value: &str) -> Result<u32, String> {
        match value.parse::<u32>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!(
                "{key} must be a positive whole number, got {value:?}"
            )),
        }
    }
    let mut params = StreamParams::default();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "w" => params.max_width = Some(positive(key, value)?),
            "h" => params.max_height = Some(positive(key, value)?),
            "q" => match value.parse::<u8>() {
                Ok(q @ 1..=100) => params.quality = q,
                _ => return Err(format!("q must be 1-100, got {value:?}")),
            },
            "fps" => match value.parse::<f32>() {
                // At least 1, so the wait between parts stays short enough
                // to notice a client leaving.
                Ok(fps) if fps.is_finite() && fps >= 1.0 => params.fps = Some(fps),
                _ => return Err(format!("fps must be a number >= 1, got {value:?}")),
            },
            "octet" => match value {
                "1" => params.octet = true,
                "0" => params.octet = false,
                _ => return Err(format!("octet must be 1 or 0, got {value:?}")),
            },
            // Ignored, like other routes' unknown keys: an `<img>` reconnects
            // with a cache-buster such as `&t=123`.
            _ => {}
        }
    }
    Ok(params)
}

/// The largest size with `width:height`'s aspect that fits `max_width` ×
/// `max_height`, never larger than the source, never zero.
pub fn fit_within(
    width: u32,
    height: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (u32, u32) {
    let scale = [
        Some(1.0),
        max_width.map(|w| w as f64 / width as f64),
        max_height.map(|h| h as f64 / height as f64),
    ]
    .into_iter()
    .flatten()
    .fold(f64::INFINITY, f64::min);
    let side = |n: u32| ((n as f64 * scale).round() as u32).clamp(1, n);
    (side(width), side(height))
}

/// Now as Unix ms, the clock `X-Capture-Ms` uses: the prelude's portable
/// clock, for the desktop render loop.
pub fn unix_ms() -> f64 {
    crate::functor_lang_prelude::now_ms()
}

/// One read-back frame, as the render loop hands it over.
pub struct StreamFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA in GL's bottom-up row order.
    pub rgba: Vec<u8>,
    /// `GET /state`'s `rendered_frames` when answered on this frame.
    pub rendered_frame: u64,
    /// Unix time of the readback, in ms.
    pub captured_ms: f64,
}

/// The render loop's side of every open stream: a request flag and a
/// latest-wins slot.
#[derive(Default)]
pub struct FrameTap {
    requested: AtomicBool,
    latest: Mutex<(u64, Option<Arc<StreamFrame>>)>,
    published: Condvar,
    /// Set when the render loop drops its [`FrameTapOwner`]: no frame will
    /// come, however many clients still hold the tap.
    closed: AtomicBool,
}

/// The render loop's hold on a [`FrameTap`]. Dropping it closes the tap and
/// wakes every waiting client, so each stream ends.
#[derive(Default)]
pub struct FrameTapOwner(Arc<FrameTap>);

impl FrameTapOwner {
    /// A client's handle on the tap.
    pub fn share(&self) -> Arc<FrameTap> {
        self.0.clone()
    }

    /// Whether only the owner holds the tap: every client has left.
    pub fn unshared(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

impl std::ops::Deref for FrameTapOwner {
    type Target = FrameTap;

    fn deref(&self) -> &FrameTap {
        &self.0
    }
}

impl Drop for FrameTapOwner {
    fn drop(&mut self) {
        // Under the lock, so a client cannot check and then miss the wake-up.
        let _latest = self.0.latest.lock().unwrap();
        self.0.closed.store(true, Ordering::Release);
        self.0.published.notify_all();
    }
}

impl FrameTap {
    /// Client side: whether the render loop is gone (see [`FrameTapOwner`]).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Render side, once per frame: whether to read this frame back. Clears
    /// the request, so one request costs at most one readback.
    pub fn take_request(&self) -> bool {
        self.requested.swap(false, Ordering::AcqRel)
    }

    /// Render side: whether a client is waiting for a frame, leaving the
    /// request in place.
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    /// Render side: replace the slot's frame and wake waiting clients.
    pub fn publish(&self, frame: StreamFrame) {
        let mut latest = self.latest.lock().unwrap();
        *latest = (latest.0 + 1, Some(Arc::new(frame)));
        self.published.notify_all();
    }

    /// Client side: ask for the next rendered frame.
    pub fn request(&self) {
        self.requested.store(true, Ordering::Release);
    }

    /// Client side: the latest frame if it was published after `seen`
    /// (0 = none yet), waiting up to `timeout` for one, or until the tap
    /// closes. Returns its sequence number to pass as `seen` next time.
    pub fn newer_than(&self, seen: u64, timeout: Duration) -> Option<(u64, Arc<StreamFrame>)> {
        let latest = self.latest.lock().unwrap();
        let (latest, _) = self
            .published
            .wait_timeout_while(latest, timeout, |(seq, _)| {
                *seq <= seen && !self.is_closed()
            })
            .unwrap();
        match &*latest {
            (seq, Some(frame)) if *seq > seen => Some((*seq, frame.clone())),
            _ => None,
        }
    }
}

/// Encode `frame` as a top-down JPEG fitted to the request's size.
pub fn encode_jpeg(frame: &StreamFrame, params: &StreamParams) -> Result<Vec<u8>, String> {
    let (width, height) = fit_within(
        frame.width,
        frame.height,
        params.max_width,
        params.max_height,
    );
    let rgba = if (width, height) == (frame.width, frame.height) {
        crate::frame_capture::flip_rgba_rows(frame.width, frame.height, &frame.rgba)?
    } else {
        downscale_flipped(frame, width, height)
    };
    let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(format!("{width}x{height} exceeds JPEG's 65535 limit"));
    };
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, params.quality)
        .encode(&rgba, w, h, jpeg_encoder::ColorType::Rgba)
        .map_err(|error| error.to_string())?;
    Ok(jpeg)
}

/// Box-filter `frame` down to `width` × `height`, flipping it top-down: each
/// output pixel averages the source pixels it covers.
fn downscale_flipped(frame: &StreamFrame, width: u32, height: u32) -> Vec<u8> {
    // Source range [start, end) that output index `i` of `out` covers in `len`.
    let span = |i: usize, out: usize, len: usize| {
        let start = i * len / out;
        (start, ((i + 1) * len / out).max(start + 1))
    };
    let (src_w, src_h) = (frame.width as usize, frame.height as usize);
    let (width, height) = (width as usize, height as usize);
    let columns: Vec<_> = (0..width).map(|x| span(x, width, src_w)).collect();
    let mut out = vec![255; width * height * 4];
    let mut sums = vec![0u64; width * 3];
    for (y, out_row) in out.chunks_exact_mut(width * 4).enumerate() {
        let (top, bottom) = span(y, height, src_h);
        sums.fill(0);
        for src_y in top..bottom {
            let row_start = (src_h - 1 - src_y) * src_w * 4;
            let row = &frame.rgba[row_start..row_start + src_w * 4];
            for (sum, &(left, right)) in sums.chunks_exact_mut(3).zip(&columns) {
                for pixel in row[left * 4..right * 4].chunks_exact(4) {
                    sum[0] += pixel[0] as u64;
                    sum[1] += pixel[1] as u64;
                    sum[2] += pixel[2] as u64;
                }
            }
        }
        for ((pixel, sum), &(left, right)) in out_row
            .chunks_exact_mut(4)
            .zip(sums.chunks_exact(3))
            .zip(&columns)
        {
            let count = ((right - left) * (bottom - top)) as u64;
            for channel in 0..3 {
                pixel[channel] = ((sum[channel] + count / 2) / count) as u8;
            }
        }
    }
    out
}

/// One part of the stream body, identical under both framings: the boundary,
/// headers, the JPEG, and a trailing CRLF.
pub fn part(jpeg: &[u8], rendered_frame: u64, captured_ms: f64) -> Vec<u8> {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\
         X-Rendered-Frame: {rendered_frame}\r\nX-Capture-Ms: {captured_ms:.1}\r\n\r\n",
        jpeg.len()
    );
    let mut part = Vec::with_capacity(head.len() + jpeg.len() + 2);
    part.extend_from_slice(head.as_bytes());
    part.extend_from_slice(jpeg);
    part.extend_from_slice(b"\r\n");
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_and_parses_every_parameter() {
        assert_eq!(parse_query(""), Ok(StreamParams::default()));
        assert_eq!(
            parse_query("w=1280&h=720&q=60&fps=30&octet=1"),
            Ok(StreamParams {
                max_width: Some(1280),
                max_height: Some(720),
                quality: 60,
                fps: Some(30.0),
                octet: true,
            })
        );
        assert_eq!(parse_query("w=640&").unwrap().max_width, Some(640));
        assert_eq!(
            parse_query("t=123"),
            Ok(StreamParams::default()),
            "cache-busters pass"
        );
        for bad in [
            "w=0",
            "h=-1",
            "q=0",
            "q=101",
            "fps=0",
            "fps=0.5",
            "fps=1e-40",
            "fps=inf",
            "octet=yes",
        ] {
            assert!(parse_query(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn content_type_and_interval_follow_the_params() {
        let multipart = StreamParams::default();
        assert_eq!(
            multipart.content_type(),
            "multipart/x-mixed-replace; boundary=functor-frame"
        );
        assert_eq!(multipart.interval(), Duration::ZERO);
        let octet = parse_query("octet=1&fps=4").unwrap();
        assert_eq!(octet.content_type(), "application/octet-stream");
        assert_eq!(octet.interval(), Duration::from_millis(250));
    }

    #[test]
    fn fit_scales_down_to_the_tighter_bound_and_never_up() {
        assert_eq!(fit_within(2560, 1440, None, None), (2560, 1440));
        assert_eq!(fit_within(2560, 1440, Some(1280), None), (1280, 720));
        assert_eq!(fit_within(2560, 1440, None, Some(360)), (640, 360));
        assert_eq!(fit_within(2560, 1440, Some(1280), Some(100)), (178, 100));
        assert_eq!(fit_within(1280, 720, Some(4000), Some(4000)), (1280, 720));
        assert_eq!(fit_within(1000, 1, Some(10), None), (10, 1));
    }

    fn frame(width: u32, height: u32, rgba: Vec<u8>) -> StreamFrame {
        StreamFrame {
            width,
            height,
            rgba,
            rendered_frame: 7,
            captured_ms: 1.5,
        }
    }

    #[test]
    fn downscale_averages_covered_pixels_and_flips_rows() {
        // Bottom-up 2x2 source rows: bottom = black, white; top = red, red.
        let source = frame(
            2,
            2,
            [
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [255, 0, 0, 255],
                [255, 0, 0, 255],
            ]
            .concat(),
        );
        assert_eq!(downscale_flipped(&source, 1, 1), vec![191, 64, 64, 255]);
        assert_eq!(
            downscale_flipped(&source, 1, 2),
            [[255, 0, 0, 255], [128, 128, 128, 255]].concat(),
            "top output row is the top source row"
        );
    }

    #[test]
    fn encodes_a_jpeg_at_the_fitted_size() {
        let source = frame(64, 32, vec![200; 64 * 32 * 4]);
        let params = StreamParams {
            max_width: Some(16),
            ..StreamParams::default()
        };
        let jpeg = encode_jpeg(&source, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (16, 8));
    }

    #[test]
    fn part_carries_its_frame_number_capture_time_and_length() {
        let part = part(b"JPEG", 42, 1700000000123.375);
        assert_eq!(
            part,
            b"--functor-frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\n\
X-Rendered-Frame: 42\r\nX-Capture-Ms: 1700000000123.4\r\n\r\nJPEG\r\n"
        );
    }

    #[test]
    fn tap_reads_back_once_per_request_and_keeps_only_the_latest_frame() {
        let tap = FrameTap::default();
        assert!(!tap.take_request(), "no client, no readback");
        tap.request();
        assert!(tap.take_request());
        assert!(!tap.take_request(), "one request, one readback");

        assert!(tap.newer_than(0, Duration::ZERO).is_none());
        tap.publish(frame(1, 1, vec![1; 4]));
        tap.publish(frame(1, 1, vec![2; 4]));
        let (seq, latest) = tap.newer_than(0, Duration::ZERO).unwrap();
        assert_eq!(
            (seq, latest.rgba[0]),
            (2, 2),
            "a slow client gets the newest frame"
        );
        assert!(
            tap.newer_than(seq, Duration::from_millis(1)).is_none(),
            "and never one twice"
        );

        let tap = Arc::new(tap);
        let publisher = {
            let tap = tap.clone();
            std::thread::spawn(move || tap.publish(frame(1, 1, vec![3; 4])))
        };
        let (seq, woken) = tap.newer_than(seq, Duration::from_secs(5)).unwrap();
        assert_eq!(
            (seq, woken.rgba[0]),
            (3, 3),
            "a waiting client wakes on publish"
        );
        publisher.join().unwrap();
    }

    /// Two clients each hold the tap, so neither can tell from the count
    /// that the render loop is gone; closing wakes and ends both.
    #[test]
    fn dropping_the_owner_closes_the_tap_for_every_client() {
        let owner = FrameTapOwner::default();
        let clients: Vec<_> = (0..2)
            .map(|_| {
                let tap = owner.share();
                std::thread::spawn(move || {
                    let started = std::time::Instant::now();
                    while !tap.is_closed() {
                        assert!(tap.newer_than(0, Duration::from_secs(30)).is_none());
                    }
                    started.elapsed()
                })
            })
            .collect();
        assert!(!owner.unshared());
        std::thread::sleep(Duration::from_millis(20));
        drop(owner);
        for client in clients {
            let waited = client.join().unwrap();
            assert!(waited < Duration::from_secs(5), "woken late: {waited:?}");
        }
    }
}
