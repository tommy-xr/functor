//! `Xr.streamDepth`: environment depth readbacks POSTed to a host URL as
//! [`crate::depth_frame`]s, off the render and sim threads.
//!
//! Bounded: one readback pending (the shared [`crate::depth_readback`]), one
//! POST in flight, and one latest frame waiting — a newer frame replaces it
//! and counts as dropped. Requests time out and are never retried, and
//! redirects are not followed. Only loopback destinations (a headset reaches
//! its host through `adb reverse`) and origins the operator allowed receive
//! frames.

use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cgmath::{Matrix4, SquareMatrix};

use crate::depth_frame::{self, DepthFrameHeader, DepthFrameSource, DepthFrameTime};
use crate::depth_readback::READBACK_SIZE;
use crate::environment_depth::EnvironmentDepthView;
pub use crate::net::http_client::Destination;
use crate::net::http_client;
use crate::runtime_settings::{DepthStreamSettings, DepthStreamState};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// The whole request: connect, send, and the response back.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// An operator's `--allow-sensor-upload` value, normalized to
/// [`Destination::origin`]'s form.
pub fn parse_origin(origin: &str) -> Result<String, String> {
    let destination = Destination::parse(origin)?;
    if destination.path != "/" {
        return Err(format!("{origin:?} must be an origin, http://host[:port], with no path"));
    }
    Ok(destination.origin())
}

/// Why `destination` may not receive frames, if it may not.
fn refusal(destination: &Destination, allowed: &[String]) -> Option<String> {
    let origin = destination.origin();
    (!destination.is_loopback() && !allowed.contains(&origin)).then(|| {
        format!(
            "Xr.streamDepth: refusing to upload sensor data to {origin}; only loopback is \
allowed by default — the operator allows another origin with `--allow-sensor-upload {origin}` \
(desktop) or `adb shell setprop debug.functor.allow_sensor_upload {origin}` (Quest)"
        )
    })
}

/// POST one body within [`REQUEST_TIMEOUT`]; any 2xx is success.
fn post(destination: &Destination, body: &[u8]) -> Result<(), String> {
    let headers = [("Content-Type".to_string(), depth_frame::CONTENT_TYPE.to_string())];
    match http_client::request("POST", destination, &headers, body, CONNECT_TIMEOUT, REQUEST_TIMEOUT)?.status {
        code if (200..300).contains(&code) => Ok(()),
        code => Err(format!("HTTP {code}")),
    }
}

/// A frame waiting for the sender thread, and the stream it belongs to.
struct Queued {
    destination: Destination,
    generation: u64,
    header: DepthFrameHeader,
    rgba: Vec<u8>,
}

/// The sender thread's state. The counters belong to the current
/// `generation`, the latest stream; an older stream's POST still in flight
/// counts nowhere.
#[derive(Default)]
struct Shared {
    generation: u64,
    latest: Option<Queued>,
    stopped: bool,
    sent: u64,
    dropped: u64,
    errors: u64,
    last_error: Option<String>,
}

type Channel = Arc<(Mutex<Shared>, Condvar)>;

fn send_thread(channel: Channel) {
    let (shared, wake) = &*channel;
    loop {
        let queued = {
            let mut shared = shared.lock().expect("depth stream state");
            loop {
                if shared.stopped {
                    return;
                }
                if let Some(queued) = shared.latest.take() {
                    break queued;
                }
                shared = wake.wait(shared).expect("depth stream state");
            }
        };
        let result = post(
            &queued.destination,
            &depth_frame::encode(&queued.header, &queued.rgba),
        );
        let mut shared = shared.lock().expect("depth stream state");
        if shared.generation != queued.generation {
            continue;
        }
        match result {
            Ok(()) => shared.sent += 1,
            Err(e) => {
                shared.errors += 1;
                shared.last_error = Some(e);
            }
        }
    }
}

/// The one sender thread every stream shares, so a new stream never puts a
/// second POST in flight beside an older stream's. Ends when dropped.
pub(crate) struct DepthSender(Channel);

impl DepthSender {
    pub(crate) fn new() -> DepthSender {
        let channel: Channel = Default::default();
        let thread_channel = channel.clone();
        std::thread::Builder::new()
            .name("functor-depth-stream".to_string())
            .spawn(move || send_thread(thread_channel))
            .expect("spawn the depth stream thread");
        DepthSender(channel)
    }
}

impl Drop for DepthSender {
    fn drop(&mut self) {
        let (shared, wake) = &*self.0;
        let mut shared = shared.lock().expect("depth stream state");
        shared.stopped = true;
        shared.latest = None;
        wake.notify_one();
    }
}

/// What a stream needs to know about one readback, from when it started.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamCapture {
    /// The stream it was taken for; another stream drops it.
    stream: u64,
    steady_ns: i64,
}

/// The nanoseconds since this process first asked: the `"steady"` clock.
fn steady_ns() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as i64
}

/// A projection's near and far planes (`None`: infinite), perspective or
/// orthographic, GL conventions.
fn near_far(projection: &Matrix4<f32>) -> (f32, Option<f32>) {
    let (a, b) = (projection.z.z, projection.w.z);
    let (near, far) = if projection.z.w == 0.0 {
        ((b + 1.0) / a, (b - 1.0) / a)
    } else {
        (b / (a - 1.0), b / (a + 1.0))
    };
    (near, (far.is_finite() && far > 0.0).then_some(far))
}

fn columns(m: Matrix4<f32>) -> [f32; 16] {
    let m: &[f32; 16] = m.as_ref();
    *m
}

/// One `settings` stream on the render thread.
pub(crate) struct DepthStreamer {
    settings: DepthStreamSettings,
    /// Where frames go and the sender's generation for this stream; `None`
    /// when refused, and `refused` says why.
    sink: Option<(Destination, Channel, u64)>,
    refused: Option<String>,
    /// The stream id, regenerated whenever streaming resumes.
    stream: u64,
    seq: u64,
    live: bool,
    /// When the next frame is due, [`steady_ns`] seconds.
    next_due: f64,
}

impl DepthStreamer {
    /// A stream to `settings.url` through `sender`, which forgets any older
    /// stream's waiting frame and counters.
    pub(crate) fn new(
        settings: DepthStreamSettings,
        allowed: &[String],
        sender: &DepthSender,
    ) -> DepthStreamer {
        let refused = match Destination::parse(&settings.url) {
            Err(e) => Err(format!("Xr.streamDepth: {e}")),
            Ok(destination) => match refusal(&destination, allowed) {
                Some(refused) => Err(refused),
                None => Ok(destination),
            },
        };
        let (sink, refused) = match refused {
            Ok(destination) => {
                let channel = sender.0.clone();
                let generation = {
                    let mut shared = channel.0.lock().expect("depth stream state");
                    *shared = Shared {
                        generation: shared.generation + 1,
                        ..Shared::default()
                    };
                    shared.generation
                };
                (Some((destination, channel, generation)), None)
            }
            Err(refused) => {
                log::warn!("{refused}");
                (None, Some(refused))
            }
        };
        DepthStreamer {
            settings,
            sink,
            refused,
            stream: 0,
            seq: 0,
            live: false,
            next_due: 0.0,
        }
    }

    /// The sender's state, while this is its current stream.
    fn shared(&self) -> Option<(std::sync::MutexGuard<'_, Shared>, &Condvar, &Destination, u64)> {
        let (destination, channel, generation) = self.sink.as_ref()?;
        let (shared, wake) = &**channel;
        let shared = shared.lock().expect("depth stream state");
        (shared.generation == *generation).then_some((shared, wake, destination, *generation))
    }

    pub(crate) fn settings(&self) -> &DepthStreamSettings {
        &self.settings
    }

    /// Stream only while `live`. Stopping discards the waiting frame;
    /// resuming starts a new stream id from `seq` 0.
    pub(crate) fn set_live(&mut self, live: bool) {
        if live && !self.live {
            self.stream = std::hash::BuildHasher::hash_one(
                &std::collections::hash_map::RandomState::new(),
                steady_ns(),
            );
            self.seq = 0;
            self.next_due = 0.0;
        }
        if !live {
            if let Some((mut shared, ..)) = self.shared() {
                shared.latest = None;
            }
        }
        self.live = live;
    }

    /// A capture to take now, if streaming and a frame is due.
    pub(crate) fn capture(&mut self) -> Option<StreamCapture> {
        if !self.live || self.sink.is_none() {
            return None;
        }
        let now = steady_ns();
        take_due(&mut self.next_due, now as f64 * 1e-9, self.settings.hz as f64).then_some(
            StreamCapture {
                stream: self.stream,
                steady_ns: now,
            },
        )
    }

    /// Queue a finished readback, replacing (and counting) a frame still
    /// waiting.
    pub(crate) fn send(
        &mut self,
        rgba: Vec<u8>,
        view: &EnvironmentDepthView,
        space_epoch: u64,
        capture: StreamCapture,
    ) {
        if !self.live || capture.stream != self.stream {
            return;
        }
        let header = self.header(view, space_epoch, capture);
        let Some((mut shared, wake, destination, generation)) = self.shared() else {
            return;
        };
        let queued = Queued {
            destination: destination.clone(),
            generation,
            header,
            rgba,
        };
        if shared.latest.replace(queued).is_some() {
            shared.dropped += 1;
        }
        wake.notify_one();
        drop(shared);
        self.seq += 1;
    }

    fn header(
        &self,
        view: &EnvironmentDepthView,
        space_epoch: u64,
        capture: StreamCapture,
    ) -> DepthFrameHeader {
        let (near, far) = near_far(&view.projection);
        let (source, clock, display_ns) = match view.display_ns {
            Some(ns) => (DepthFrameSource::Device, "xr", ns),
            None => (DepthFrameSource::Emulated, "steady", capture.steady_ns),
        };
        DepthFrameHeader {
            schema: depth_frame::SCHEMA.to_string(),
            stream: format!("{:016x}", self.stream),
            seq: self.seq,
            source,
            // The first pass of a frame reads back: the left eye in stereo.
            eye: "left".to_string(),
            width: READBACK_SIZE,
            height: READBACK_SIZE,
            encoding: depth_frame::ENCODING.to_string(),
            rows: depth_frame::ROWS.to_string(),
            camera_to_world: columns(view.view.invert().unwrap_or(Matrix4::identity())),
            projection: columns(view.projection),
            view_projection: columns(view.view_projection),
            near,
            far,
            // Shells render (and so read back) only while tracking is valid.
            tracking: true,
            space_epoch,
            time: DepthFrameTime {
                clock: clock.to_string(),
                display_ns,
                acquire_ns: None,
                sensor_ns: None,
            },
            payload_len: (READBACK_SIZE * READBACK_SIZE * 4) as u64,
        }
    }

    /// `GET /state.xr.stream`.
    pub(crate) fn state(&self) -> DepthStreamState {
        let mut state = DepthStreamState {
            url: self.settings.url.clone(),
            hz: self.settings.hz,
            last_error: self.refused.clone(),
            ..Default::default()
        };
        if let Some((shared, ..)) = self.shared() {
            state.sent = shared.sent;
            state.dropped = shared.dropped;
            state.errors = shared.errors;
            state.last_error = shared.last_error.clone();
        }
        state
    }
}

impl Drop for DepthStreamer {
    /// A stopped stream's waiting frame is discarded; a POST in flight
    /// finishes.
    fn drop(&mut self) {
        if let Some((mut shared, ..)) = self.shared() {
            shared.latest = None;
        }
    }
}

/// Whether a frame is due at `now` (seconds) at `hz`, advancing `next_due`.
/// A tenth of a period's slack absorbs frame-time jitter, so readbacks every
/// 33 ms still stream 15 Hz, and the average rate holds where `hz` falls
/// between readbacks. After a stall (or at the start) the schedule restarts
/// from `now`, with no catch-up burst.
fn take_due(next_due: &mut f64, now: f64, hz: f64) -> bool {
    let period = 1.0 / hz;
    if now < *next_due - period * 0.1 {
        return false;
    }
    *next_due = if now - *next_due > period {
        now + period
    } else {
        *next_due + period
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_parse() {
        assert_eq!(parse_origin("http://192.168.1.5:9000").unwrap(), "http://192.168.1.5:9000");
        assert!(parse_origin("http://192.168.1.5:9000/frames").is_err());
    }

    #[test]
    fn loopback_is_allowed_and_other_origins_need_the_operator() {
        let allowed = vec!["http://192.168.1.5:9000".to_string()];
        let refused = |url: &str| refusal(&Destination::parse(url).unwrap(), &allowed);
        assert_eq!(refused("http://127.0.0.1:9000/f"), None);
        assert_eq!(refused("http://localhost:1/"), None);
        assert_eq!(refused("http://[::1]/"), None);
        assert_eq!(refused("http://192.168.1.5:9000/frames"), None);
        assert!(refused("http://192.168.1.5:9001/frames").is_some());
        assert!(refused("http://example.com/").is_some());
        // A refused streamer starts no thread and says why.
        let streamer = DepthStreamer::new(
            DepthStreamSettings {
                url: "http://10.0.0.2/f".to_string(),
                hz: 10.0,
            },
            &[],
            &DepthSender::new(),
        );
        assert!(streamer.sink.is_none());
        assert!(streamer.state().last_error.unwrap().contains("--allow-sensor-upload"));
    }

    #[test]
    fn the_rate_holds_between_readbacks() {
        // Readbacks every 33.3 ms with jitter, over 3 s.
        let count = |hz: f64| {
            let mut next_due = 0.0;
            (0..90)
                .filter(|i| {
                    let jitter = if i % 2 == 0 { 0.0 } else { 0.001 };
                    take_due(&mut next_due, *i as f64 / 30.0 + jitter, hz)
                })
                .count()
        };
        assert_eq!(count(15.0), 45);
        assert_eq!(count(30.0), 90);
        assert!((59..=61).contains(&count(20.0)));
        assert!((14..=16).contains(&count(5.0)));
        assert_eq!(count(1000.0), 90);
        // A stale schedule (a new stream, a stall) restarts without a burst.
        let mut next_due = 0.0;
        let sent: Vec<f64> = (300..360)
            .map(|i| i as f64 / 30.0)
            .filter(|now| take_due(&mut next_due, *now, 1.0))
            .collect();
        assert_eq!(sent.len(), 3);
        assert!(sent.windows(2).all(|w| w[1] - w[0] > 0.89), "{sent:?}");
    }

    #[test]
    fn near_and_far_come_from_the_projection() {
        let finite = cgmath::perspective(cgmath::Deg(90.0), 1.0, 0.1, 20.0);
        let (near, far) = near_far(&finite);
        assert!((near - 0.1).abs() < 1e-5 && (far.unwrap() - 20.0).abs() < 1e-2);
        let mut camera = crate::Camera::default();
        camera.near = 0.25;
        camera.far = f32::INFINITY;
        let infinite = camera.projection_matrix_from_fov_angles(-0.7, 0.7, -0.7, 0.7);
        assert_eq!(near_far(&infinite), (0.25, None));
        let ortho = cgmath::ortho(-1.0, 1.0, -1.0, 1.0, 0.5, 8.0);
        let (near, far) = near_far(&ortho);
        assert!((near - 0.5).abs() < 1e-5 && (far.unwrap() - 8.0).abs() < 1e-4);
    }
}
