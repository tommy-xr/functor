//! `Xr.streamDepth` and `Xr.streamCamera`: sensor frames POSTed to a host
//! URL, off the render and sim threads.
//!
//! Bounded per stream kind: one POST in flight and one latest frame waiting —
//! a newer frame replaces it and counts as dropped. Requests time out and are
//! never retried, and redirects are not followed. Only loopback destinations
//! (a headset reaches its host through `adb reverse`) and origins the
//! operator allowed receive frames.

use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use crate::net::http_client::Destination;
use crate::net::http_client;
use crate::runtime_settings::{SensorStreamSettings, SensorStreamState};

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

/// Why `destination` may not receive `knob`'s frames, if it may not.
fn refusal(knob: &str, destination: &Destination, allowed: &[String]) -> Option<String> {
    let origin = destination.origin();
    (!destination.is_loopback() && !allowed.contains(&origin)).then(|| {
        format!(
            "{knob}: refusing to upload sensor data to {origin}; only loopback is \
allowed by default — the operator allows another origin with `--allow-sensor-upload {origin}` \
(desktop) or `adb shell setprop debug.functor.allow_sensor_upload {origin}` (Quest)"
        )
    })
}

/// POST one frame — a big-endian u32 header length, the header, the payload
/// — within [`REQUEST_TIMEOUT`]; any 2xx is success. The response body is
/// never read.
fn post(
    destination: &Destination,
    content_type: &str,
    header: &[u8],
    payload: &[u8],
) -> Result<(), String> {
    let mut body = Vec::with_capacity(4 + header.len() + payload.len());
    body.extend_from_slice(&(header.len() as u32).to_be_bytes());
    body.extend_from_slice(header);
    body.extend_from_slice(payload);
    let headers = [("Content-Type".to_string(), content_type.to_string())];
    match http_client::request_status("POST", destination, &headers, &body, CONNECT_TIMEOUT, REQUEST_TIMEOUT)? {
        code if (200..300).contains(&code) => Ok(()),
        code => Err(format!("HTTP {code}")),
    }
}

/// A frame waiting for the sender thread, and the stream it belongs to.
struct Queued {
    destination: Destination,
    generation: u64,
    content_type: &'static str,
    /// The JSON header.
    header: Vec<u8>,
    /// Builds the payload on the sender thread (a conversion stays off the
    /// render thread).
    payload: Box<dyn FnOnce() -> Vec<u8> + Send>,
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
            let mut shared = shared.lock().expect("sensor stream state");
            loop {
                if shared.stopped {
                    return;
                }
                if let Some(queued) = shared.latest.take() {
                    break queued;
                }
                shared = wake.wait(shared).expect("sensor stream state");
            }
        };
        let payload = (queued.payload)();
        let result = post(
            &queued.destination,
            queued.content_type,
            &queued.header,
            &payload,
        );
        let mut shared = shared.lock().expect("sensor stream state");
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

/// The one sender thread every stream of a kind (depth, camera) shares, so a
/// new stream never puts a second POST in flight beside an older stream's.
/// Ends when dropped.
pub(crate) struct SensorSender(Channel);

impl SensorSender {
    /// `kind` names the thread: "depth", "camera".
    pub(crate) fn new(kind: &str) -> SensorSender {
        let channel: Channel = Default::default();
        let thread_channel = channel.clone();
        std::thread::Builder::new()
            .name(format!("functor-{kind}-stream"))
            .spawn(move || send_thread(thread_channel))
            .expect("spawn the sensor stream thread");
        SensorSender(channel)
    }
}

impl Drop for SensorSender {
    fn drop(&mut self) {
        let (shared, wake) = &*self.0;
        let mut shared = shared.lock().expect("sensor stream state");
        shared.stopped = true;
        shared.latest = None;
        wake.notify_one();
    }
}

/// A frame a stream asked for ([`SensorStreamer::capture`]), carried to
/// [`SensorStreamer::send`] with whatever produced it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamCapture {
    /// The stream it was taken for; another stream drops it.
    stream: u64,
    /// When it was asked for, [`steady_ns`].
    pub(crate) steady_ns: i64,
}

/// A frame's place in its stream, for its header.
pub(crate) struct FrameIds {
    /// 16 hex digits, new each time the stream starts.
    pub(crate) stream: String,
    /// +1 per frame queued; a gap is a dropped frame.
    pub(crate) seq: u64,
}

/// The nanoseconds since this process first asked: the `"steady"` clock.
fn steady_ns() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as i64
}

/// One `settings` stream (`knob`'s) on the render thread.
pub(crate) struct SensorStreamer {
    knob: &'static str,
    content_type: &'static str,
    settings: SensorStreamSettings,
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

impl SensorStreamer {
    /// `knob`'s stream of `content_type` frames to `settings.url` through
    /// `sender`, which forgets any older stream's waiting frame and counters.
    pub(crate) fn new(
        knob: &'static str,
        content_type: &'static str,
        settings: SensorStreamSettings,
        allowed: &[String],
        sender: &SensorSender,
    ) -> SensorStreamer {
        let refused = match Destination::parse(&settings.url) {
            Err(e) => Err(format!("{knob}: {e}")),
            Ok(destination) => match refusal(knob, &destination, allowed) {
                Some(refused) => Err(refused),
                None => Ok(destination),
            },
        };
        let (sink, refused) = match refused {
            Ok(destination) => {
                let channel = sender.0.clone();
                let generation = {
                    let mut shared = channel.0.lock().expect("sensor stream state");
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
        SensorStreamer {
            knob,
            content_type,
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
        let shared = shared.lock().expect("sensor stream state");
        (shared.generation == *generation).then_some((shared, wake, destination, *generation))
    }

    pub(crate) fn settings(&self) -> &SensorStreamSettings {
        &self.settings
    }

    /// Stream only while `live`. Stopping discards the waiting frame;
    /// resuming starts a new stream id from `seq` 0.
    pub(crate) fn set_live(&mut self, live: bool) {
        if live && !self.live {
            self.stream = std::hash::BuildHasher::hash_one(
                &std::collections::hash_map::RandomState::new(),
                (self.knob, steady_ns()),
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

    /// A frame to take now, if streaming and one is due.
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

    /// Queue `capture`'s frame — `header` serializes its JSON header given its
    /// ids; `payload` runs on the sender thread — replacing (and counting) a
    /// frame still waiting. A capture from before a stop or restart is
    /// dropped.
    pub(crate) fn send(
        &mut self,
        capture: StreamCapture,
        payload: impl FnOnce() -> Vec<u8> + Send + 'static,
        header: impl FnOnce(FrameIds) -> Vec<u8>,
    ) {
        if !self.live || capture.stream != self.stream {
            return;
        }
        let header = header(FrameIds {
            stream: format!("{:016x}", self.stream),
            seq: self.seq,
        });
        let content_type = self.content_type;
        let Some((mut shared, wake, destination, generation)) = self.shared() else {
            return;
        };
        let queued = Queued {
            destination: destination.clone(),
            generation,
            content_type,
            header,
            payload: Box::new(payload),
        };
        if shared.latest.replace(queued).is_some() {
            shared.dropped += 1;
        }
        wake.notify_one();
        drop(shared);
        self.seq += 1;
    }

    /// `GET /state`'s report of this stream.
    pub(crate) fn state(&self) -> SensorStreamState {
        let mut state = SensorStreamState {
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

impl Drop for SensorStreamer {
    /// A stopped stream's waiting frame is discarded; a POST in flight
    /// finishes.
    fn drop(&mut self) {
        if let Some((mut shared, ..)) = self.shared() {
            shared.latest = None;
        }
    }
}

/// Follow `settings`: `None` stops `slot`'s stream; new settings start a new
/// one through `sender` (created on first use). Frames go out only while
/// `live`.
pub(crate) fn follow(
    slot: &mut Option<SensorStreamer>,
    sender: &mut Option<SensorSender>,
    kind: &str,
    knob: &'static str,
    content_type: &'static str,
    settings: Option<&SensorStreamSettings>,
    allowed: &[String],
    live: bool,
) {
    let Some(settings) = settings else {
        *slot = None;
        return;
    };
    if slot.as_ref().is_none_or(|stream| stream.settings() != settings) {
        *slot = Some(SensorStreamer::new(
            knob,
            content_type,
            settings.clone(),
            allowed,
            sender.get_or_insert_with(|| SensorSender::new(kind)),
        ));
    }
    slot.as_mut().expect("set above").set_live(live);
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
        let refused = |url: &str| refusal("Xr.streamDepth", &Destination::parse(url).unwrap(), &allowed);
        assert_eq!(refused("http://127.0.0.1:9000/f"), None);
        assert_eq!(refused("http://localhost:1/"), None);
        assert_eq!(refused("http://[::1]/"), None);
        assert_eq!(refused("http://192.168.1.5:9000/frames"), None);
        assert!(refused("http://192.168.1.5:9001/frames").is_some());
        assert!(refused("http://example.com/").is_some());
        // A refused streamer starts no thread and says why.
        let streamer = SensorStreamer::new(
            "Xr.streamDepth",
            "application/octet-stream",
            SensorStreamSettings {
                url: "http://10.0.0.2/f".to_string(),
                hz: 10.0,
            },
            &[],
            &SensorSender::new("depth"),
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
}
