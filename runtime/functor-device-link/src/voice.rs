//! Voice feedback: a spoken clip, headset → desktop, and the desktop's
//! progress on it, desktop → headset. Both ride [`Frame::Channel`].
//!
//! - [`CLIP_CHANNEL`] carries [`Clip`] messages for one utterance at a
//!   time: `Start` (format and the [capsule](Capsule)), `Chunk`s of 16 kHz
//!   mono 16-bit little-endian PCM numbered from 0, then `End` (or
//!   `Cancel`).
//! - [`STATUS_CHANNEL`] carries [`Status`]: `Heard` → `Transcribed` →
//!   `Working` → `Reloaded`, or `Failed` at any point; `Idle` clears.
//!
//! Every message starts with [`VERSION`]; a peer of another version is
//! refused with [`Error::Version`]. `decode` refuses anything over the limits
//! below, so a receiver can trust the sizes. Big-endian, like [`crate::wire`].

use crate::wire::{allowed, clip, Frame, Reader, Writer};
use crate::Error;

/// The first byte of every voice message.
pub const VERSION: u8 = 1;

/// Headset → desktop: [`Clip`] messages.
pub const CLIP_CHANNEL: u16 = 1;
/// Desktop → headset: [`Status`] messages.
pub const STATUS_CHANNEL: u16 = 2;

/// The one format this version carries: 16 kHz mono 16-bit PCM, what
/// speech-to-text takes without resampling.
pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u8 = 1;
/// Longest utterance.
pub const MAX_SECONDS: u32 = 60;
/// Most audio bytes in one utterance: [`MAX_SECONDS`] of the format.
pub const MAX_CLIP: usize = MAX_SECONDS as usize * SAMPLE_RATE as usize * 2;
/// Most audio bytes in one `Chunk` (about a second), so each fits one
/// [`crate::MAX_MESSAGE`] with room to spare.
pub const MAX_CHUNK: usize = 32 * 1024;
/// Most bytes of capsule JSON.
pub const MAX_CAPSULE: usize = 16 * 1024;
/// Most bytes of a transcript in `Transcribed`; longer is cut.
pub const MAX_TEXT: usize = 1024;
/// Most bytes of a reason in `Failed`; longer is cut.
pub const MAX_REASON: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pcm16Le,
}

/// One message of a spoken clip, headset → desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clip {
    /// A new utterance; `utterance` is the headset's, unique per link.
    Start {
        utterance: u64,
        sample_rate: u32,
        channels: u8,
        format: Format,
        /// UTF-8 JSON of a [`Capsule`]; may be empty.
        capsule: Vec<u8>,
    },
    /// Audio, `seq` counting from 0 with no gaps.
    Chunk {
        utterance: u64,
        seq: u32,
        data: Vec<u8>,
    },
    /// The user let go: the clip is complete.
    End { utterance: u64 },
    /// The user cancelled: drop the clip.
    Cancel { utterance: u64 },
}

impl Clip {
    pub fn utterance(&self) -> u64 {
        match self {
            Clip::Start { utterance, .. }
            | Clip::Chunk { utterance, .. }
            | Clip::End { utterance }
            | Clip::Cancel { utterance } => *utterance,
        }
    }

    /// The bytes, or `TooLarge` for a chunk or capsule over its limit.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::default();
        w.u8(VERSION);
        match self {
            Clip::Start {
                utterance,
                sample_rate,
                channels,
                format: Format::Pcm16Le,
                capsule,
            } => {
                if capsule.len() > MAX_CAPSULE {
                    return Err(Error::TooLarge);
                }
                w.u8(1);
                w.u64(*utterance);
                w.u32(*sample_rate);
                w.u8(*channels);
                w.u8(0);
                w.blob(capsule);
            }
            Clip::Chunk {
                utterance,
                seq,
                data,
            } => {
                if data.len() > MAX_CHUNK {
                    return Err(Error::TooLarge);
                }
                w.u8(2);
                w.u64(*utterance);
                w.u32(*seq);
                w.blob(data);
            }
            Clip::End { utterance } => {
                w.u8(3);
                w.u64(*utterance);
            }
            Clip::Cancel { utterance } => {
                w.u8(4);
                w.u64(*utterance);
            }
        }
        Ok(w.0)
    }

    /// As a frame on [`CLIP_CHANNEL`].
    pub fn frame(&self) -> Result<Frame, Error> {
        Ok(Frame::Channel {
            channel: CLIP_CHANNEL,
            data: self.encode()?,
        })
    }

    /// Refuses other versions, formats other than [`SAMPLE_RATE`] mono
    /// `Pcm16Le`, a chunk over [`MAX_CHUNK`], a capsule over [`MAX_CAPSULE`]
    /// or not UTF-8.
    pub fn decode(bytes: &[u8]) -> Result<Clip, Error> {
        let mut r = Reader::new(bytes);
        version(&mut r)?;
        let clip = match r.u8()? {
            1 => {
                let utterance = r.u64()?;
                let (sample_rate, channels, format) = (r.u32()?, r.u8()?, r.u8()?);
                if sample_rate != SAMPLE_RATE || channels != CHANNELS || format != 0 {
                    return Err(Error::Malformed("voice format"));
                }
                let capsule = blob(&mut r, MAX_CAPSULE)?;
                if std::str::from_utf8(&capsule).is_err() {
                    return Err(Error::Malformed("capsule"));
                }
                Clip::Start {
                    utterance,
                    sample_rate,
                    channels,
                    format: Format::Pcm16Le,
                    capsule,
                }
            }
            2 => Clip::Chunk {
                utterance: r.u64()?,
                seq: r.u32()?,
                data: blob(&mut r, MAX_CHUNK)?,
            },
            3 => Clip::End {
                utterance: r.u64()?,
            },
            4 => Clip::Cancel {
                utterance: r.u64()?,
            },
            _ => return Err(Error::Malformed("voice message kind")),
        };
        r.end()?;
        Ok(clip)
    }
}

/// Where the desktop is with an utterance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The clip arrived whole.
    Heard,
    /// What it said.
    Transcribed {
        text: String,
    },
    /// An agent run started or was steered with it.
    Working,
    /// The game on the headset reloaded with the run's edit.
    Reloaded,
    Failed {
        reason: String,
    },
    /// Nothing in progress: clear the toast.
    Idle,
}

/// The desktop's progress, desktop → headset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The utterance it is about; `None` for `Idle` or a run not started by
    /// voice.
    pub utterance: Option<u64>,
    pub stage: Stage,
}

impl Status {
    /// The bytes. Text is cut to its limit and characters a peer refuses
    /// (control, bidi) become spaces, so every status decodes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(VERSION);
        match self.utterance {
            Some(u) => {
                w.u8(1);
                w.u64(u);
            }
            None => w.u8(0),
        }
        match &self.stage {
            Stage::Heard => w.u8(1),
            Stage::Transcribed { text } => {
                w.u8(2);
                w.str16(&tidy(text, MAX_TEXT));
            }
            Stage::Working => w.u8(3),
            Stage::Reloaded => w.u8(4),
            Stage::Failed { reason } => {
                w.u8(5);
                w.str16(&tidy(reason, MAX_REASON));
            }
            Stage::Idle => w.u8(6),
        }
        w.0
    }

    /// As a frame on [`STATUS_CHANNEL`].
    pub fn frame(&self) -> Frame {
        Frame::Channel {
            channel: STATUS_CHANNEL,
            data: self.encode(),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Status, Error> {
        let mut r = Reader::new(bytes);
        version(&mut r)?;
        let utterance = match r.flag()? {
            true => Some(r.u64()?),
            false => None,
        };
        let stage = match r.u8()? {
            1 => Stage::Heard,
            2 => Stage::Transcribed {
                text: r.text16(MAX_TEXT)?,
            },
            3 => Stage::Working,
            4 => Stage::Reloaded,
            5 => Stage::Failed {
                reason: r.text16(MAX_REASON)?,
            },
            6 => Stage::Idle,
            _ => return Err(Error::Malformed("voice stage")),
        };
        r.end()?;
        Ok(Status { utterance, stage })
    }
}

fn version(r: &mut Reader) -> Result<(), Error> {
    match r.u8()? {
        VERSION => Ok(()),
        theirs => Err(Error::Version {
            theirs: theirs.into(),
            ours: VERSION.into(),
        }),
    }
}

fn blob(r: &mut Reader, max: usize) -> Result<Vec<u8>, Error> {
    let len = r.u32()? as usize;
    if len > max {
        return Err(Error::TooLarge);
    }
    Ok(r.take(len)?.to_vec())
}

/// `text` with refused characters as spaces, cut to `max` bytes.
fn tidy(text: &str, max: usize) -> String {
    let text: String = text
        .chars()
        .map(|c| match allowed(c.encode_utf8(&mut [0; 4])) {
            true => c,
            false => ' ',
        })
        .collect();
    clip(&text, max).to_string()
}

/// What the user was looking at when they spoke: JSON in `Clip::Start`.
/// Every field is optional and unknown fields are ignored, so either side
/// can grow it.
#[cfg(feature = "capsule")]
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Capsule {
    /// `/state`'s `frame` when the user started speaking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
    /// `/state`'s `tts` (game seconds) then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<Pose>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Pose>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Pose>,
    /// What the user pointed at, by the game's name for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_at: Option<String>,
}

/// A pose in the game's world, as `/state`'s xr input reports one.
#[cfg(feature = "capsule")]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pose {
    pub position: [f32; 3],
    /// Quaternion `[x, y, z, w]`.
    pub orientation: [f32; 4],
}

#[cfg(feature = "capsule")]
impl Capsule {
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a capsule serializes")
    }

    /// `None` for empty or unreadable JSON: a capsule is a hint, never a
    /// reason to drop the utterance.
    pub fn from_json(bytes: &[u8]) -> Option<Capsule> {
        serde_json::from_slice(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(capsule: &[u8]) -> Clip {
        Clip::Start {
            utterance: 7,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            format: Format::Pcm16Le,
            capsule: capsule.to_vec(),
        }
    }

    #[test]
    fn every_message_round_trips() {
        let clips = [
            start(b""),
            start(br#"{"frame":3}"#),
            Clip::Chunk {
                utterance: 7,
                seq: 0,
                data: vec![1; MAX_CHUNK],
            },
            Clip::End { utterance: 7 },
            Clip::Cancel {
                utterance: u64::MAX,
            },
        ];
        for clip in clips {
            assert_eq!(Clip::decode(&clip.encode().unwrap()).unwrap(), clip);
        }
        let stages = [
            Stage::Heard,
            Stage::Transcribed {
                text: "make the ball red".into(),
            },
            Stage::Working,
            Stage::Reloaded,
            Stage::Failed {
                reason: "no project".into(),
            },
            Stage::Idle,
        ];
        for (i, stage) in stages.into_iter().enumerate() {
            let status = Status {
                utterance: (i % 2 == 0).then_some(i as u64),
                stage,
            };
            assert_eq!(Status::decode(&status.encode()).unwrap(), status);
        }
    }

    #[test]
    fn a_whole_chunk_fits_one_link_message() {
        let frame = Clip::Chunk {
            utterance: 1,
            seq: 1,
            data: vec![0; MAX_CHUNK],
        };
        // Noise adds a 16-byte tag; leave room for it.
        assert!(frame.frame().unwrap().encode().len() + 16 < crate::MAX_MESSAGE);
    }

    #[test]
    fn decoding_enforces_limits_and_format() {
        // Encode refuses what decode would.
        let big = Clip::Chunk {
            utterance: 1,
            seq: 0,
            data: vec![0; MAX_CHUNK + 1],
        };
        assert_eq!(big.encode(), Err(Error::TooLarge));
        assert_eq!(
            start(&[b' '; MAX_CAPSULE + 1]).encode(),
            Err(Error::TooLarge)
        );

        // A forged oversize chunk: its length prefix is over the limit.
        let mut bytes = Clip::Chunk {
            utterance: 1,
            seq: 0,
            data: vec![0; 4],
        }
        .encode()
        .unwrap();
        let at = bytes.len() - 8;
        bytes[at..at + 4].copy_from_slice(&(MAX_CHUNK as u32 + 1).to_be_bytes());
        assert_eq!(Clip::decode(&bytes), Err(Error::TooLarge));

        let wrong = |rate: u32, channels: u8, format: u8| {
            let mut bytes = start(b"").encode().unwrap();
            bytes[10..14].copy_from_slice(&rate.to_be_bytes());
            bytes[14] = channels;
            bytes[15] = format;
            Clip::decode(&bytes)
        };
        assert!(wrong(SAMPLE_RATE, CHANNELS, 0).is_ok());
        for (rate, channels, format) in [(44_100, 1, 0), (SAMPLE_RATE, 2, 0), (SAMPLE_RATE, 1, 1)] {
            assert_eq!(
                wrong(rate, channels, format),
                Err(Error::Malformed("voice format"))
            );
        }

        assert_eq!(
            Clip::decode(&start(&[0xff]).encode().unwrap()),
            Err(Error::Malformed("capsule"))
        );
        assert!(Clip::decode(&[]).is_err());
        assert!(Clip::decode(&[VERSION, 9]).is_err());
        let mut trailing = Clip::End { utterance: 1 }.encode().unwrap();
        trailing.push(0);
        assert_eq!(
            Clip::decode(&trailing),
            Err(Error::Malformed("trailing bytes"))
        );

        let mut long = vec![VERSION, 0, 2];
        long.extend_from_slice(&(MAX_TEXT as u16 + 1).to_be_bytes());
        long.extend(std::iter::repeat_n(b'a', MAX_TEXT + 1));
        assert_eq!(Status::decode(&long), Err(Error::TooLarge));
    }

    #[test]
    fn another_version_is_refused_with_ours() {
        let mut bytes = Clip::End { utterance: 1 }.encode().unwrap();
        bytes[0] = 2;
        let version = Error::Version { theirs: 2, ours: 1 };
        assert_eq!(Clip::decode(&bytes), Err(version.clone()));
        let mut bytes = Status {
            utterance: None,
            stage: Stage::Idle,
        }
        .encode();
        bytes[0] = 2;
        assert_eq!(Status::decode(&bytes), Err(version));
    }

    #[test]
    fn status_text_is_cut_and_cleaned_so_it_always_decodes() {
        let status = Status {
            utterance: Some(1),
            stage: Stage::Transcribed {
                text: format!("line\none\u{202e}{}", "é".repeat(MAX_TEXT)),
            },
        };
        let Stage::Transcribed { text } = Status::decode(&status.encode()).unwrap().stage else {
            panic!("a transcript");
        };
        assert!(text.starts_with("line one é"));
        assert!(text.len() <= MAX_TEXT);
    }

    #[cfg(feature = "capsule")]
    #[test]
    fn a_capsule_round_trips_and_tolerates_unknown_or_bad_json() {
        let capsule = Capsule {
            frame: Some(1234),
            tts: Some(20.5),
            head: Some(Pose {
                position: [0.0, 1.6, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            }),
            pointing_at: Some("crate".into()),
            ..Capsule::default()
        };
        assert_eq!(Capsule::from_json(&capsule.to_json()), Some(capsule));
        let later = br#"{"frame": 5, "gaze": [1, 2, 3]}"#;
        assert_eq!(Capsule::from_json(later).unwrap().frame, Some(5));
        assert_eq!(Capsule::from_json(b""), None);
        assert_eq!(Capsule::from_json(b"not json"), None);
    }
}
