//! A mirror producer (`--mirror <url>`): draws another session's frames —
//! a headset's, through its device-link tunnel — instead of running game
//! logic. It polls the source's `GET /mirror` (the frame on screen and the
//! view it was drawn from), one request at a time at most `fps` times a
//! second, and only while someone sees it (a visible window, a capture, or a
//! client streaming a hidden session), so an unwatched mirror costs its
//! source nothing. Each frame draws from the source's view
//! (the head, on a headset); the shell's debug camera (`POST /camera`)
//! overrides it like any other authored camera. Input is ignored: the
//! source owns the game.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use functor_runtime_common::debug_protocol::{MirrorSnapshot, MirrorState};
use functor_runtime_common::ui::View;
use functor_runtime_common::{Frame, FrameTime};

use crate::game::Game;
#[cfg(test)]
use functor_runtime_common::{Camera, Scene3D};

/// A request's limit: the tunnel answers within its own 30 s, but a mirror
/// that waits that long shows nothing useful.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Between tries while the source does not answer.
const RETRY: Duration = Duration::from_secs(1);
/// How often an unwatched poller checks whether it is watched again.
const IDLE: Duration = Duration::from_millis(100);

pub struct MirrorGame {
    shared: Arc<Shared>,
    /// The snapshot `render_at` last drew.
    shown: Option<MirrorSnapshot>,
}

struct Shared {
    source: String,
    watched: AtomicBool,
    stop: AtomicBool,
    latest: Mutex<Latest>,
}

#[derive(Default)]
struct Latest {
    snapshot: Option<MirrorSnapshot>,
    frames: u64,
    /// Arrival times in the last second, for `fps`.
    arrivals: VecDeque<Instant>,
    latency_ms: Option<f32>,
    error: Option<String>,
}

impl MirrorGame {
    /// Mirrors the debug server at `source` (`http://host:port`), polling it
    /// at most `fps` times a second while watched.
    pub fn create(source: &str, fps: f32) -> MirrorGame {
        let shared = Arc::new(Shared {
            source: source.trim_end_matches('/').to_string(),
            watched: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            latest: Mutex::default(),
        });
        let interval = Duration::from_secs_f32(1.0 / fps.clamp(1.0, 120.0));
        let poller = shared.clone();
        std::thread::Builder::new()
            .name("functor-mirror".to_string())
            .spawn(move || poll(&poller, interval))
            .expect("spawn the mirror poller");
        eprintln!("[mirror] mirroring {source} at up to {fps} fps while watched");
        MirrorGame {
            shared,
            shown: None,
        }
    }
}

impl Drop for MirrorGame {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

fn poll(shared: &Shared, interval: Duration) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            shared.latest.lock().unwrap().error = Some(format!("no HTTP client: {error}"));
            return;
        }
    };
    let url = format!("{}/mirror", shared.source);
    while !shared.stop.load(Ordering::Acquire) {
        if !shared.watched.load(Ordering::Acquire) {
            std::thread::sleep(IDLE);
            continue;
        }
        let started = Instant::now();
        let result = fetch(&client, &url);
        let now = Instant::now();
        let mut latest = shared.latest.lock().unwrap();
        latest.latency_ms = Some(now.duration_since(started).as_secs_f32() * 1000.0);
        let wait = match result {
            Ok(snapshot) => {
                latest.snapshot = Some(snapshot);
                latest.frames += 1;
                latest.arrivals.push_back(now);
                latest.error = None;
                interval.saturating_sub(now.duration_since(started))
            }
            Err(error) => {
                latest.error = Some(error);
                RETRY
            }
        };
        drop(latest);
        std::thread::sleep(wait);
    }
}

fn fetch(client: &reqwest::blocking::Client, url: &str) -> Result<MirrorSnapshot, String> {
    let response = client.get(url).send().map_err(|e| {
        if e.is_timeout() {
            "the source did not answer in time".to_string()
        } else {
            format!("cannot reach the source: {e}")
        }
    })?;
    let status = response.status();
    let body = response
        .bytes()
        .map_err(|e| format!("cannot read the frame: {e}"))?;
    match status.as_u16() {
        200 => serde_json::from_slice(&body).map_err(|e| format!("not a mirror frame: {e}")),
        404 => Err("it runs a Functor too old to mirror (no GET /mirror); update it".to_string()),
        _ => Err(format!(
            "{status}: {}",
            String::from_utf8_lossy(&body[..body.len().min(256)]).trim()
        )),
    }
}

impl Game for MirrorGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {}

    fn tick(&mut self, _frame_time: FrameTime) {}

    fn key_event(&mut self, _code: i32, _is_down: bool) {}
    fn mouse_move(&mut self, _x: i32, _y: i32) {}
    fn mouse_wheel(&mut self, _delta: i32) {}

    /// The newest snapshot, viewed from where its source viewed it.
    fn render_at(&mut self, _frame_time: FrameTime, _alpha: f32) -> Frame {
        if let Some(snapshot) = self.shared.latest.lock().unwrap().snapshot.take() {
            self.shown = Some(snapshot);
        }
        match &self.shown {
            Some(snapshot) => {
                let mut frame = Frame::clone(&snapshot.scene);
                if let Some(view) = &snapshot.view {
                    frame.camera = view.clone();
                }
                frame
            }
            // Nothing yet: an empty world, until the first snapshot lands.
            None => crate::functor_lang_game::empty_frame(),
        }
    }

    /// The source's clock for its frame, so render-time animation matches.
    fn render_time(&self, shell: FrameTime) -> FrameTime {
        match &self.shown {
            Some(snapshot) => FrameTime {
                tts: snapshot.tts,
                dts: shell.dts,
            },
            None => shell,
        }
    }

    /// One more per snapshot drawn or waiting, so a paused, quiet shell
    /// redraws when one arrives.
    fn model_revision(&self) -> u64 {
        self.shared.latest.lock().unwrap().frames
    }

    fn set_watched(&mut self, watched: bool) {
        self.shared.watched.store(watched, Ordering::Release);
    }

    fn mirror_state(&self) -> Option<MirrorState> {
        let mut latest = self.shared.latest.lock().unwrap();
        let now = Instant::now();
        while latest
            .arrivals
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
        {
            latest.arrivals.pop_front();
        }
        let shown = latest.snapshot.as_ref().or(self.shown.as_ref());
        Some(MirrorState {
            source: self.shared.source.clone(),
            watched: self.shared.watched.load(Ordering::Acquire),
            frames: latest.frames,
            fps: latest.arrivals.len() as f32,
            latency_ms: latest.latency_ms,
            source_frame: shown.map(|s| s.rendered_frames),
            source_tts: shown.map(|s| s.tts),
            view: shown.and_then(|s| s.view.clone()),
            error: latest.error.clone(),
        })
    }

    fn ui(&self) -> View {
        View::empty()
    }

    fn state_debug(&self) -> String {
        format!("<mirror of {}>", self.shared.source)
    }

    fn net_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn net_push_http_response(&mut self, _token: i32, _status: i32, _body: String) {}
    fn net_push_http_error(&mut self, _token: i32, _message: String) {}
    fn audio_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn audio_scene_json(&self) -> String {
        "{\"sources\":[]}".to_string()
    }
    fn net_drain_conn_commands(&self) -> String {
        "[]".to_string()
    }
    fn net_push_connected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_message(&mut self, _key: String, _conn: i32, _text: String) {}
    fn net_push_disconnected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_error(&mut self, _key: String, _conn: i32, _message: String) {}
    fn audio_push_finished(&mut self, _token: i32) {}

    fn quit(&mut self) {}
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicU64;

    use super::*;

    /// A source answering every request with `status` and `body`; counts them.
    fn source(status: &'static str, body: String) -> (String, Arc<AtomicU64>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicU64::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                seen.fetch_add(1, Ordering::SeqCst);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (url, count)
    }

    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn polls_only_while_watched_and_draws_from_the_source_view() {
        let view = Camera {
            eye: [1.0, 1.6, 2.0],
            ..Camera::default()
        };
        let snapshot = MirrorSnapshot {
            rendered_frames: 42,
            tts: 3.0,
            view: Some(view.clone()),
            scene: Arc::new(Frame::new(Camera::default(), Scene3D::cube())),
        };
        let (url, requests) = source("200 OK", serde_json::to_string(&snapshot).unwrap());
        let mut mirror = MirrorGame::create(&url, 100.0);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(requests.load(Ordering::SeqCst), 0, "unwatched: no polls");

        mirror.set_watched(true);
        until("two snapshots", || mirror.model_revision() >= 2);
        let frame = mirror.render_at(FrameTime { tts: 0.0, dts: 0.0 }, 0.0);
        assert_eq!(frame.camera, view);
        assert_eq!(frame.scene, snapshot.scene.scene);
        let state = mirror.mirror_state().unwrap();
        assert!(state.watched && state.fps >= 1.0 && state.error.is_none());
        assert_eq!((state.source, state.source_frame), (url, Some(42)));

        mirror.set_watched(false);
        std::thread::sleep(Duration::from_millis(100));
        let stopped = requests.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            stopped,
            "unwatched again: polls stop"
        );
    }

    #[test]
    fn reports_a_source_that_cannot_mirror() {
        let (url, _) = source("404 Not Found", "not found".to_string());
        let mut mirror = MirrorGame::create(&url, 100.0);
        mirror.set_watched(true);
        until("an error", || {
            mirror.mirror_state().unwrap().error.is_some()
        });
        let state = mirror.mirror_state().unwrap();
        assert_eq!(
            state.error.as_deref(),
            Some("it runs a Functor too old to mirror (no GET /mirror); update it")
        );
        assert_eq!(state.frames, 0);
    }
}
