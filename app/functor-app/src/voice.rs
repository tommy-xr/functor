//! Voice from a headset, the shell's half (plan 4.2/4.3): a clip assembled
//! from a link's voice channel (bounded, one utterance at a time), turned
//! into text here, and its capsule rendered for the agent. The audio stays
//! in memory and is dropped once transcribed, unless
//! `FUNCTOR_APP_KEEP_AUDIO` is set (then `<home>/voice/<device>-<utterance>.wav`).

use std::sync::Arc;

use functor_device_link::voice::{self, Capsule, Clip, Pose, Stage, Status, MAX_CLIP};

/// A clip that stops arriving this long is dropped.
pub const CLIP_TIMEOUT_MS: u64 = 10_000;

/// What one voice message asks of the connection.
#[derive(Debug, PartialEq)]
pub enum Heard {
    /// Send this to the headset.
    Status(Status),
    /// Utterance `utterance` is complete: transcribe it.
    Clip {
        utterance: u64,
        pcm: Vec<i16>,
        capsule: String,
    },
}

/// One link's clip in progress, and the utterance being transcribed.
#[derive(Default)]
pub struct Assembler {
    clip: Option<Partial>,
    transcribing: Option<u64>,
    /// `transcribing` was cancelled: its text is dropped.
    cancelled: bool,
    /// The utterance last refused: its later messages are dropped quietly.
    refused: Option<u64>,
}

struct Partial {
    utterance: u64,
    capsule: String,
    audio: Vec<u8>,
    next: u32,
    /// When its last message came.
    at: u64,
}

impl Assembler {
    /// Refuses `utterance` (and drops its clip), saying why.
    fn refuse(&mut self, utterance: Option<u64>, reason: &str) -> Option<Heard> {
        if self.clip.as_ref().map(|c| c.utterance) == utterance {
            self.clip = None;
        }
        self.refused = utterance;
        Some(failed(utterance, reason))
    }
}

fn failed(utterance: Option<u64>, reason: &str) -> Heard {
    Heard::Status(Status {
        utterance,
        stage: Stage::Failed {
            reason: reason.to_string(),
        },
    })
}

impl Assembler {
    /// Takes one message from the voice channel at `now` (ms).
    pub fn receive(&mut self, data: &[u8], now: u64) -> Option<Heard> {
        let msg = match Clip::decode(data) {
            Ok(msg) => msg,
            Err(e) => {
                let current = self.clip.as_ref().map(|c| c.utterance);
                return self.refuse(current, &format!("an unreadable voice message: {e}"));
            }
        };
        let u = msg.utterance();
        let ours = self.clip.as_ref().is_some_and(|c| c.utterance == u);
        match msg {
            Clip::Start { .. } if self.clip.is_some() || self.transcribing.is_some() => {
                self.refuse(Some(u), "still working on the last request")
            }
            Clip::Start { capsule, .. } => {
                self.clip = Some(Partial {
                    utterance: u,
                    capsule,
                    audio: Vec::new(),
                    next: 0,
                    at: now,
                });
                None
            }
            Clip::Chunk { seq, data, .. } if ours => {
                let clip = self.clip.as_mut().expect("ours");
                if seq != clip.next {
                    return self.refuse(Some(u), "part of the recording went missing");
                }
                if clip.audio.len() + data.len() > MAX_CLIP {
                    let reason = format!("longer than {} seconds", voice::MAX_SECONDS);
                    return self.refuse(Some(u), &reason);
                }
                clip.audio.extend_from_slice(&data);
                clip.next += 1;
                clip.at = now;
                None
            }
            Clip::End { .. } if ours => {
                let clip = self.clip.take().expect("ours");
                if clip.audio.len() < 2 {
                    return self.refuse(Some(u), "heard nothing");
                }
                self.transcribing = Some(u);
                let pcm = clip
                    .audio
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| i16::from_le_bytes(*s))
                    .collect();
                Some(Heard::Clip {
                    utterance: u,
                    pcm,
                    capsule: clip.capsule,
                })
            }
            Clip::Cancel { .. } if ours || self.transcribing == Some(u) => {
                self.clip = None;
                self.cancelled = self.transcribing == Some(u);
                Some(Heard::Status(Status {
                    utterance: Some(u),
                    stage: Stage::Idle,
                }))
            }
            // The rest of one refused.
            _ if self.refused == Some(u) => None,
            // Another utterance's chunk or end: not one being heard.
            _ => self.refuse(Some(u), "that recording is not being heard"),
        }
    }

    /// Utterance `utterance` is transcribed (or failed to be): whether its
    /// text still counts (it was not cancelled meanwhile).
    pub fn transcribed(&mut self, utterance: u64) -> bool {
        if self.transcribing != Some(utterance) {
            return true;
        }
        self.transcribing = None;
        !std::mem::take(&mut self.cancelled)
    }

    /// A clip that stopped arriving, dropped.
    pub fn check(&mut self, now: u64) -> Option<Heard> {
        let stale = self
            .clip
            .as_ref()
            .is_some_and(|c| now.saturating_sub(c.at) >= CLIP_TIMEOUT_MS);
        if !stale {
            return None;
        }
        let u = self.clip.as_ref().map(|c| c.utterance);
        self.refuse(u, "the recording stopped arriving")
    }
}

/// The capsule as a phrase for the agent: "at frame 1234 (20.5 s in), head
/// at (0.00, 1.60, 0.00) looking along (0.00, 0.00, -1.00), pointing at
/// \"crate\"". `None` when it is empty or unreadable.
pub fn render(json: &str) -> Option<String> {
    let c = Capsule::from_json(json)?;
    // `+ 0.0` turns -0.0 into 0.0, which prints without a sign.
    let at = |p: &[f32; 3]| format!("({:.2}, {:.2}, {:.2})", p[0] + 0.0, p[1] + 0.0, p[2] + 0.0);
    let mut parts = Vec::new();
    match (c.frame, c.tts) {
        (Some(f), Some(t)) => parts.push(format!("at frame {f} ({t:.1} s in)")),
        (Some(f), None) => parts.push(format!("at frame {f}")),
        (None, Some(t)) => parts.push(format!("{t:.1} s in")),
        (None, None) => {}
    }
    if let Some(head) = c.head {
        let look = forward(head.orientation);
        parts.push(format!(
            "head at {} looking along {}",
            at(&head.position),
            at(&look)
        ));
    }
    for (hand, pose) in [("left", c.left), ("right", c.right)] {
        if let Some(p) = pose {
            parts.push(format!("{hand} hand at {}", at(&p.position)));
        }
    }
    if let Some(name) = c.pointing_at.filter(|n| !n.trim().is_empty()) {
        parts.push(format!("pointing at {name:?}"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// A capsule from a debug runtime's `GET /state`: its frame and game time,
/// and the tracked head and controller grips when it reports xr input.
pub fn capsule_from_state(state: &serde_json::Value) -> Capsule {
    let nums = |v: &serde_json::Value| -> Option<Vec<f32>> {
        v.as_array()?
            .iter()
            .map(|n| n.as_f64().map(|n| n as f32))
            .collect()
    };
    let pose = |v: &serde_json::Value| -> Option<Pose> {
        Some(Pose {
            position: nums(&v["position"])?.try_into().ok()?,
            orientation: nums(&v["orientation"])?.try_into().ok()?,
        })
    };
    let xr = &state["input"]["xr"];
    Capsule {
        frame: state["frame"].as_u64(),
        tts: state["tts"].as_f64(),
        head: pose(&xr["head"]),
        left: pose(&xr["left"]["grip"]),
        right: pose(&xr["right"]["grip"]),
        pointing_at: None,
    }
}

/// Where a pose looks: its -Z axis (OpenXR's forward), in rig space.
fn forward([x, y, z, w]: [f32; 4]) -> [f32; 3] {
    [
        -2.0 * (x * z + w * y),
        -2.0 * (y * z - w * x),
        -(1.0 - 2.0 * (x * x + y * y)),
    ]
}

/// Turns speech into text, here.
#[derive(Clone)]
pub enum Speech {
    /// `FUNCTOR_APP_FAKE_TRANSCRIPT`: every clip says this (tests).
    Fake(Arc<functor_stt::Fake>),
    /// whisper.cpp, loaded (and its model fetched) on first use.
    #[cfg(feature = "whisper")]
    Whisper(Arc<tokio::sync::OnceCell<Arc<functor_stt::Whisper>>>),
    /// Not in this build.
    #[cfg(not(feature = "whisper"))]
    Off,
}

impl Speech {
    pub fn from_env() -> Speech {
        if let Some(text) = std::env::var_os("FUNCTOR_APP_FAKE_TRANSCRIPT") {
            let text = text.to_string_lossy().into_owned();
            return Speech::Fake(Arc::new(functor_stt::Fake(text)));
        }
        #[cfg(feature = "whisper")]
        return Speech::Whisper(Arc::default());
        #[cfg(not(feature = "whisper"))]
        Speech::Off
    }

    /// Whether this app can turn speech into text.
    pub fn available(&self) -> bool {
        cfg!(feature = "whisper") || matches!(self, Speech::Fake(_))
    }

    /// The text spoken in `pcm` (16 kHz mono), off the async threads.
    pub async fn transcribe(&self, pcm: Vec<i16>) -> Result<String, String> {
        use functor_stt::Transcriber;
        let model: Arc<dyn Transcriber> = match self {
            Speech::Fake(fake) => fake.clone(),
            #[cfg(feature = "whisper")]
            Speech::Whisper(cell) => cell
                .get_or_try_init(|| async {
                    let load = || {
                        let dir = functor_stt::models_dir(
                            std::env::var_os("FUNCTOR_MODELS"),
                            std::env::home_dir(),
                        )
                        .ok_or("no home directory for the speech model; set FUNCTOR_MODELS")?;
                        let path = functor_stt::ensure_model(&dir, &functor_stt::DEFAULT_MODEL)?;
                        functor_stt::Whisper::load(&path).map(Arc::new)
                    };
                    tokio::task::spawn_blocking(load)
                        .await
                        .map_err(|e| e.to_string())?
                })
                .await?
                .clone(),
            #[cfg(not(feature = "whisper"))]
            Speech::Off => {
                return Err(
                    "speech-to-text is not in this build of the app: build it with \
`--features whisper` (as `npm run build:app` does)"
                        .to_string(),
                )
            }
        };
        tokio::task::spawn_blocking(move || model.transcribe(&pcm))
            .await
            .map_err(|e| e.to_string())?
    }
}

/// With `FUNCTOR_APP_KEEP_AUDIO` set, keeps `pcm` as a WAV under `home`
/// for debugging; otherwise nothing is written.
pub fn keep_audio(home: &std::path::Path, name: &str, pcm: &[i16]) {
    if std::env::var_os("FUNCTOR_APP_KEEP_AUDIO").is_none() {
        return;
    }
    let dir = home.join("voice");
    let path = dir.join(format!("{name}.wav"));
    let kept = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, functor_stt::write_wav(pcm)));
    if let Err(e) = kept {
        eprintln!("functor-app: cannot keep {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(clip: Clip) -> Vec<u8> {
        clip.encode().unwrap()
    }

    fn start(u: u64) -> Vec<u8> {
        bytes(Clip::Start {
            utterance: u,
            capsule: r#"{"frame":3}"#.into(),
        })
    }

    fn chunk(u: u64, seq: u32, data: Vec<u8>) -> Vec<u8> {
        bytes(Clip::Chunk {
            utterance: u,
            seq,
            data,
        })
    }

    fn failed_with(heard: Option<Heard>) -> (Option<u64>, String) {
        match heard {
            Some(Heard::Status(Status {
                utterance,
                stage: Stage::Failed { reason },
            })) => (utterance, reason),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_clip_is_assembled_in_order_then_one_at_a_time() {
        let mut a = Assembler::default();
        assert_eq!(a.receive(&start(1), 0), None);
        assert_eq!(a.receive(&chunk(1, 0, vec![1, 0, 2, 0]), 1), None);
        assert_eq!(a.receive(&chunk(1, 1, vec![3, 0]), 2), None);
        let done = a.receive(&bytes(Clip::End { utterance: 1 }), 3);
        assert_eq!(
            done,
            Some(Heard::Clip {
                utterance: 1,
                pcm: vec![1, 2, 3],
                capsule: r#"{"frame":3}"#.into(),
            })
        );
        // While it is transcribed, another waits its turn: refused.
        assert_eq!(failed_with(a.receive(&start(2), 4)).0, Some(2));
        assert!(a.transcribed(1));
        assert_eq!(a.receive(&start(3), 5), None);
    }

    #[test]
    fn gaps_oversize_garbage_strangers_and_silence_fail_the_clip() {
        let mut a = Assembler::default();
        a.receive(&start(1), 0);
        assert_eq!(
            failed_with(a.receive(&chunk(1, 1, vec![0, 0]), 0)),
            (Some(1), "part of the recording went missing".into())
        );

        a.receive(&start(2), 0);
        let full = voice::MAX_CHUNK;
        let mut seq = 0;
        let over = loop {
            match a.receive(&chunk(2, seq, vec![0; full]), 0) {
                None => seq += 1,
                some => break some,
            }
        };
        assert_eq!(failed_with(over).0, Some(2));
        assert!(seq as usize * full <= MAX_CLIP);
        // The rest of it is dropped quietly.
        assert_eq!(a.receive(&chunk(2, seq + 1, vec![0; 2]), 0), None);
        assert_eq!(a.receive(&bytes(Clip::End { utterance: 2 }), 0), None);

        a.receive(&start(3), 0);
        let (u, reason) = failed_with(a.receive(b"junk", 0));
        assert_eq!(u, Some(3));
        assert!(
            reason.starts_with("an unreadable voice message"),
            "{reason}"
        );

        assert_eq!(
            failed_with(a.receive(&bytes(Clip::End { utterance: 9 }), 0)).0,
            Some(9)
        );

        a.receive(&start(4), 0);
        let (_, reason) = failed_with(a.receive(&bytes(Clip::End { utterance: 4 }), 0));
        assert_eq!(reason, "heard nothing");
    }

    #[test]
    fn a_cancel_clears_and_a_stalled_clip_times_out() {
        let mut a = Assembler::default();
        a.receive(&start(1), 0);
        let idle = a.receive(&bytes(Clip::Cancel { utterance: 1 }), 0);
        assert_eq!(
            idle,
            Some(Heard::Status(Status {
                utterance: Some(1),
                stage: Stage::Idle
            }))
        );
        // Cancelled while transcribed: idle, and its text is dropped.
        a.receive(&start(5), 0);
        a.receive(&chunk(5, 0, vec![1, 0]), 0);
        a.receive(&bytes(Clip::End { utterance: 5 }), 0);
        let idle = a.receive(&bytes(Clip::Cancel { utterance: 5 }), 0);
        assert!(matches!(
            idle,
            Some(Heard::Status(Status {
                stage: Stage::Idle,
                ..
            }))
        ));
        assert!(!a.transcribed(5));
        a.receive(&start(2), 100);
        assert_eq!(a.check(100 + CLIP_TIMEOUT_MS - 1), None);
        assert_eq!(failed_with(a.check(100 + CLIP_TIMEOUT_MS)).0, Some(2));
        assert_eq!(a.receive(&start(3), 0), None);
    }

    #[test]
    fn a_capsule_is_built_from_a_sessions_state() {
        let state = serde_json::json!({
            "frame": 42, "tts": 0.7,
            "input": {"xr": {
                "head": {"position": [0.0, 1.6, 0.0], "orientation": [0.0, 0.0, 0.0, 1.0]},
                "left": {"active": true},
                "right": {"active": true, "grip": {"position": [0.2, 1.0, -0.3], "orientation": [0.0, 0.0, 0.0, 1.0]}}
            }}
        });
        let c = capsule_from_state(&state);
        assert_eq!((c.frame, c.tts), (Some(42), Some(0.7)));
        assert_eq!(c.head.unwrap().position, [0.0, 1.6, 0.0]);
        assert_eq!(c.left, None);
        assert_eq!(c.right.unwrap().position, [0.2, 1.0, -0.3]);
        // A desktop game: no xr, so frame and time only.
        let flat = capsule_from_state(&serde_json::json!({"frame": 3, "tts": 0.05, "input": {}}));
        assert_eq!(
            render(&flat.to_json()).as_deref(),
            Some("at frame 3 (0.1 s in)")
        );
    }

    #[test]
    fn a_capsule_renders_as_a_phrase() {
        let json = r#"{"frame":1234,"tts":20.5,
            "head":{"position":[0,1.6,0],"orientation":[0,0,0,1]},
            "right":{"position":[0.2,1.1,-0.3],"orientation":[0,0,0,1]},
            "pointing_at":"crate"}"#;
        assert_eq!(
            render(json).as_deref(),
            Some(
                "at frame 1234 (20.5 s in), head at (0.00, 1.60, 0.00) looking along \
(0.00, 0.00, -1.00), right hand at (0.20, 1.10, -0.30), pointing at \"crate\""
            )
        );
        assert_eq!(render(""), None);
        assert_eq!(render("{}"), None);
    }
}
