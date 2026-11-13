//! Push-to-talk on the headset: microphone samples in, one utterance's
//! [`Clip`] messages out (`Start`, `Chunk`s of [`CHUNK`] bytes, then `End`
//! or `Cancel`). Pure: the shell owns the microphone, the clock and the link,
//! and nothing is kept once a message is handed to the [`Sink`].

use crate::voice::{Clip, Format, CHANNELS, MAX_CLIP, SAMPLE_RATE};

/// Audio bytes per `Chunk`: 250 ms.
pub const CHUNK: usize = SAMPLE_RATE as usize / 4 * 2;

/// Where the messages go: the link's queue.
pub trait Sink {
    /// Queues `clip`; `false` if refused: not linked, or `audio` and the
    /// link's queue is full (it is not keeping up).
    fn send(&mut self, clip: Clip, audio: bool) -> bool;
}

/// How an utterance ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// Whole, ended by `End`.
    Sent,
    /// The user cancelled; `Cancel` was sent.
    Cancelled,
    /// The link's queue was full; `Cancel` was sent.
    TooSlow,
    /// The link dropped; nothing more is sent.
    Lost,
}

#[derive(Default)]
pub struct Talk {
    next: u64,
    open: Option<Open>,
}

struct Open {
    utterance: u64,
    seq: u32,
    /// Audio not yet a whole chunk.
    pending: Vec<u8>,
    /// Audio bytes taken so far, chunked or pending.
    taken: usize,
    resampler: Resampler,
    level: f32,
}

impl Talk {
    /// Utterance ids count up from `first` (unique per link: the shell
    /// seeds it from the clock).
    pub fn new(first: u64) -> Talk {
        Talk {
            next: first,
            open: None,
        }
    }

    /// Starts an utterance of audio sampled at `rate`; `None` if the link
    /// refused its `Start`. Ends any open one first, as cancelled.
    pub fn start(&mut self, rate: u32, capsule: Vec<u8>, sink: &mut impl Sink) -> Option<u64> {
        self.cancel(sink);
        let utterance = self.next;
        self.next += 1;
        let start = Clip::Start {
            utterance,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            format: Format::Pcm16Le,
            capsule,
        };
        if !sink.send(start, false) {
            return None;
        }
        self.open = Some(Open {
            utterance,
            seq: 0,
            pending: Vec::with_capacity(CHUNK),
            taken: 0,
            resampler: Resampler::new(rate),
            level: 0.0,
        });
        Some(utterance)
    }

    pub fn utterance(&self) -> Option<u64> {
        self.open.as_ref().map(|o| o.utterance)
    }

    /// The last samples' loudness, 0 (−60 dBFS or less) to 1 (full scale).
    pub fn level(&self) -> f32 {
        self.open.as_ref().map_or(0.0, |o| o.level)
    }

    /// Microphone samples at the rate given to [`Talk::start`]. Sends each
    /// whole chunk; ends the utterance when the link refuses one, or with
    /// `End` once it holds [`crate::voice::MAX_SECONDS`] of audio.
    pub fn push(&mut self, samples: &[i16], sink: &mut impl Sink) -> Option<Stopped> {
        let open = self.open.as_mut()?;
        if !samples.is_empty() {
            open.level = level(samples);
        }
        let mut resampled = Vec::with_capacity(samples.len());
        open.resampler.push(samples, &mut resampled);
        let room = (MAX_CLIP - open.taken) / 2;
        for s in resampled.iter().take(room) {
            open.pending.extend_from_slice(&s.to_le_bytes());
        }
        open.taken += resampled.len().min(room) * 2;
        while self.open.as_ref()?.pending.len() >= CHUNK {
            let pending = &mut self.open.as_mut()?.pending;
            let rest = pending.split_off(CHUNK);
            let data = std::mem::replace(pending, rest);
            if !self.chunk(data, sink) {
                return self.refused(sink);
            }
        }
        if self.open.as_ref()?.taken >= MAX_CLIP {
            return self.finish(sink);
        }
        None
    }

    /// The user let go: the rest of the audio, then `End`.
    pub fn finish(&mut self, sink: &mut impl Sink) -> Option<Stopped> {
        let data = std::mem::take(&mut self.open.as_mut()?.pending);
        if !data.is_empty() && !self.chunk(data, sink) {
            return self.refused(sink);
        }
        let open = self.open.take()?;
        Some(match sink.send(
            Clip::End {
                utterance: open.utterance,
            },
            false,
        ) {
            true => Stopped::Sent,
            false => Stopped::Lost,
        })
    }

    /// The user cancelled.
    pub fn cancel(&mut self, sink: &mut impl Sink) -> Option<Stopped> {
        let open = self.open.take()?;
        sink.send(
            Clip::Cancel {
                utterance: open.utterance,
            },
            false,
        );
        Some(Stopped::Cancelled)
    }

    /// The link dropped: forget the utterance; nothing is resent.
    pub fn lost(&mut self) -> Option<Stopped> {
        self.open.take().map(|_| Stopped::Lost)
    }

    fn chunk(&mut self, data: Vec<u8>, sink: &mut impl Sink) -> bool {
        let open = self.open.as_mut().expect("open");
        let clip = Clip::Chunk {
            utterance: open.utterance,
            seq: open.seq,
            data,
        };
        open.seq += 1;
        sink.send(clip, true)
    }

    /// A chunk was refused: cancel, unless the link is gone altogether.
    fn refused(&mut self, sink: &mut impl Sink) -> Option<Stopped> {
        let open = self.open.take()?;
        let cancel = Clip::Cancel {
            utterance: open.utterance,
        };
        Some(match sink.send(cancel, false) {
            true => Stopped::TooSlow,
            false => Stopped::Lost,
        })
    }
}

/// RMS of `samples` on a −60..0 dBFS scale, as 0..1.
fn level(samples: &[i16]) -> f32 {
    let mean = samples.iter().map(|&s| (s as f32).powi(2)).sum::<f32>() / samples.len() as f32;
    let db = 20.0 * (mean.sqrt() / i16::MAX as f32).max(1e-6).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Mono samples at any rate to [`SAMPLE_RATE`], averaging each output
/// sample's span of input (48 kHz: every three), carrying the remainder
/// across calls.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output sample's span starts in `carry`.
    at: f64,
    carry: Vec<i16>,
}

impl Resampler {
    fn new(rate: u32) -> Resampler {
        Resampler {
            step: rate.max(1) as f64 / SAMPLE_RATE as f64,
            at: 0.0,
            carry: Vec::new(),
        }
    }

    fn push(&mut self, input: &[i16], out: &mut Vec<i16>) {
        if self.step == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        self.carry.extend_from_slice(input);
        while self.at + self.step <= self.carry.len() as f64 {
            let from = self.at as usize;
            let to = ((self.at + self.step) as usize).max(from + 1);
            let span = &self.carry[from..to];
            out.push((span.iter().map(|&s| s as i32).sum::<i32>() / span.len() as i32) as i16);
            self.at += self.step;
        }
        let used = self.at as usize;
        self.carry.drain(..used);
        self.at -= used as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::{MAX_CHUNK, MAX_SECONDS};

    /// The link: what it took, refusing audio past `room` messages and
    /// everything once `linked` is off.
    struct Link {
        sent: Vec<Clip>,
        room: usize,
        linked: bool,
    }

    impl Link {
        fn new() -> Link {
            Link {
                sent: Vec::new(),
                room: usize::MAX,
                linked: true,
            }
        }

        /// The audio bytes it took for `utterance`, checking `seq` has no gaps.
        fn audio(&self, utterance: u64) -> Vec<u8> {
            let mut next = 0;
            let mut audio = Vec::new();
            for clip in &self.sent {
                if let Clip::Chunk {
                    utterance: u,
                    seq,
                    data,
                } = clip
                {
                    if *u == utterance {
                        assert_eq!(*seq, next);
                        assert!(data.len() <= MAX_CHUNK);
                        next += 1;
                        audio.extend(data);
                    }
                }
            }
            audio
        }
    }

    impl Sink for Link {
        fn send(&mut self, clip: Clip, audio: bool) -> bool {
            if !self.linked || (audio && self.sent.len() >= self.room) {
                return false;
            }
            self.sent.push(clip);
            true
        }
    }

    /// A fake microphone: a 440 Hz tone at `rate`, in callbacks of `n`.
    fn mic(rate: u32, seconds: f64, n: usize) -> Vec<Vec<i16>> {
        let total = (rate as f64 * seconds).round() as usize;
        let tone: Vec<i16> = (0..total)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (8000.0 * (t * 440.0 * std::f32::consts::TAU).sin()) as i16
            })
            .collect();
        tone.chunks(n).map(<[i16]>::to_vec).collect()
    }

    #[test]
    fn a_held_button_becomes_start_chunks_and_end() {
        let mut link = Link::new();
        let mut talk = Talk::new(100);
        let id = talk.start(48_000, b"{\"frame\":1}".to_vec(), &mut link).unwrap();
        assert_eq!(id, 100);
        for callback in mic(48_000, 1.3, 480) {
            assert_eq!(talk.push(&callback, &mut link), None);
        }
        assert!(talk.level() > 0.5, "a loud tone: {}", talk.level());
        assert_eq!(talk.finish(&mut link), Some(Stopped::Sent));
        assert!(matches!(&link.sent[0], Clip::Start { utterance: 100, capsule, .. } if capsule == b"{\"frame\":1}"));
        assert_eq!(link.sent.last(), Some(&Clip::End { utterance: 100 }));
        // 1.3 s at 48 kHz is 1.3 s at 16 kHz: five whole chunks and the rest.
        assert_eq!(link.audio(100).len(), 20_800 * 2);
        assert_eq!(link.sent.len(), 1 + 6 + 1);
        // Every message is one the desktop decodes.
        for clip in &link.sent {
            assert_eq!(&Clip::decode(&clip.encode().unwrap()).unwrap(), clip);
        }
        assert_eq!(talk.utterance(), None);
        // The next utterance has its own id.
        assert_eq!(talk.start(16_000, Vec::new(), &mut link), Some(101));
    }

    #[test]
    fn resampling_keeps_the_tone() {
        let mut link = Link::new();
        let mut talk = Talk::new(1);
        talk.start(48_000, Vec::new(), &mut link);
        for callback in mic(48_000, 0.5, 333) {
            talk.push(&callback, &mut link);
        }
        talk.finish(&mut link);
        let pcm: Vec<i16> = link
            .audio(1)
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(pcm.len(), 8000);
        // 440 Hz: 880 zero crossings a second, so about 440 in half a second.
        let crossings = pcm.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!((430..=450).contains(&crossings), "{crossings}");
    }

    #[test]
    fn it_ends_by_itself_at_the_limit() {
        let mut link = Link::new();
        let mut talk = Talk::new(1);
        talk.start(16_000, Vec::new(), &mut link);
        let mut stopped = None;
        for callback in mic(16_000, MAX_SECONDS as f64 + 2.0, 1600) {
            stopped = stopped.or(talk.push(&callback, &mut link));
        }
        assert_eq!(stopped, Some(Stopped::Sent));
        assert_eq!(link.audio(1).len(), MAX_CLIP);
        assert_eq!(link.sent.last(), Some(&Clip::End { utterance: 1 }));
    }

    #[test]
    fn a_slow_link_cancels_and_a_dropped_one_sends_nothing_more() {
        let mut link = Link::new();
        link.room = 3;
        let mut talk = Talk::new(1);
        talk.start(16_000, Vec::new(), &mut link);
        let stopped = mic(16_000, 2.0, 1600)
            .iter()
            .find_map(|callback| talk.push(callback, &mut link));
        assert_eq!(stopped, Some(Stopped::TooSlow));
        assert_eq!(link.sent.last(), Some(&Clip::Cancel { utterance: 1 }));
        assert_eq!(talk.utterance(), None);

        let mut link = Link::new();
        talk.start(16_000, Vec::new(), &mut link);
        talk.push(&mic(16_000, 0.5, 8000)[0], &mut link);
        link.linked = false;
        let sent = link.sent.len();
        assert_eq!(talk.lost(), Some(Stopped::Lost));
        assert_eq!(talk.finish(&mut link), None);
        assert_eq!(link.sent.len(), sent, "nothing resent");
        // Refused at the start: no utterance opens.
        assert_eq!(talk.start(16_000, Vec::new(), &mut link), None);
        assert_eq!(talk.utterance(), None);
    }

    #[test]
    fn cancelling_sends_cancel() {
        let mut link = Link::new();
        let mut talk = Talk::new(1);
        talk.start(16_000, Vec::new(), &mut link);
        talk.push(&mic(16_000, 0.3, 4800)[0], &mut link);
        assert_eq!(talk.cancel(&mut link), Some(Stopped::Cancelled));
        assert_eq!(link.sent.last(), Some(&Clip::Cancel { utterance: 1 }));
        assert_eq!(talk.cancel(&mut link), None);
    }
}
