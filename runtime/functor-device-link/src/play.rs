//! A spoken reply on the headset: [`Speech`] messages in, samples for the
//! audio output out. Pure: the shell owns the output stream and the link.
//! One reply at a time: a `Start` replaces the reply before it, and a gap,
//! another reply's audio or more than [`MAX_SECONDS`] drops it.

use crate::talk::Resampler;
use crate::voice::{Speech, MAX_SECONDS};

#[derive(Default)]
pub struct Player {
    open: Option<Open>,
    /// At the output's rate; played up to `at`.
    samples: Vec<i16>,
    at: usize,
}

/// The reply arriving.
struct Open {
    reply: u64,
    seq: u32,
    /// Audio bytes it may still send.
    room: usize,
    resampler: Resampler,
}

impl Player {
    /// The next message from the desktop, for an output of `rate` samples
    /// per second.
    pub fn receive(&mut self, speech: Speech, rate: u32) {
        match speech {
            Speech::Start { reply, rate: from } => {
                self.stop();
                self.open = Some(Open {
                    reply,
                    seq: 0,
                    room: MAX_SECONDS as usize * from as usize * 2,
                    resampler: Resampler::new(from, rate),
                });
            }
            Speech::Chunk { reply, seq, data } => {
                let Some(open) = &mut self.open else {
                    return;
                };
                let whole = data.len() % 2 == 0 && data.len() <= open.room;
                if reply != open.reply || seq != open.seq || !whole {
                    return self.stop();
                }
                open.seq += 1;
                open.room -= data.len();
                let samples: Vec<i16> = data
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                open.resampler.push(&samples, &mut self.samples);
            }
            // What arrived plays out.
            Speech::End { reply } => {
                if self.open.as_ref().is_some_and(|o| o.reply == reply) {
                    self.open = None;
                }
            }
            Speech::Stop { reply } => {
                // An ended reply still playing is the one last started.
                if self.open.as_ref().is_none_or(|o| o.reply == reply) {
                    self.stop();
                }
            }
        }
    }

    /// Silence: the user started talking, or the reply was dropped.
    pub fn stop(&mut self) {
        *self = Player::default();
    }

    /// A reply is arriving or has samples left to play.
    pub fn playing(&self) -> bool {
        self.open.is_some() || !self.pending().is_empty()
    }

    /// The samples not yet played; [`Player::played`] takes them.
    pub fn pending(&self) -> &[i16] {
        &self.samples[self.at..]
    }

    /// The output took the first `count` of [`Player::pending`].
    pub fn played(&mut self, count: usize) {
        self.at = (self.at + count).min(self.samples.len());
        // Played out: free it.
        if self.open.is_none() && self.at == self.samples.len() {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::SAMPLE_RATE;

    const RATE: u32 = SAMPLE_RATE;

    fn start(reply: u64) -> Speech {
        Speech::Start { reply, rate: RATE }
    }

    fn chunk(reply: u64, seq: u32, samples: &[i16]) -> Speech {
        Speech::Chunk {
            reply,
            seq,
            data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
        }
    }

    fn player(speech: impl IntoIterator<Item = Speech>) -> Player {
        let mut player = Player::default();
        for speech in speech {
            player.receive(speech, RATE);
        }
        player
    }

    #[test]
    fn a_reply_plays_in_order_as_it_arrives_and_ends_when_played_out() {
        let mut player = player([start(1), chunk(1, 0, &[1, 2, 3])]);
        assert_eq!(player.pending(), [1, 2, 3]);
        // The output took two; more arrives.
        player.played(2);
        player.receive(chunk(1, 1, &[4]), RATE);
        assert_eq!(player.pending(), [3, 4]);
        player.played(2);
        assert!(player.playing(), "more may come");
        player.receive(Speech::End { reply: 1 }, RATE);
        assert!(!player.playing());

        // Ended with samples left: they play out.
        let mut player = player_ended();
        assert!(player.playing());
        player.played(9);
        assert!(!player.playing() && player.pending().is_empty());
    }

    fn player_ended() -> Player {
        player([start(1), chunk(1, 0, &[1, 2]), Speech::End { reply: 1 }])
    }

    #[test]
    fn a_gap_another_replys_audio_or_too_much_drops_it() {
        let dropped = [
            chunk(1, 2, &[9]),
            chunk(2, 1, &[9]),
            Speech::Chunk {
                reply: 1,
                seq: 1,
                data: vec![0; 3],
            },
        ];
        for bad in dropped {
            let mut player = player([start(1), chunk(1, 0, &[1, 2]), bad.clone()]);
            assert!(!player.playing(), "{bad:?}");
            // What follows of it is not played either.
            player.receive(chunk(1, 1, &[3]), RATE);
            assert!(player.pending().is_empty());
        }
        // Audio with no reply started.
        assert!(!player([chunk(1, 0, &[1])]).playing());

        let mut long = player([start(1)]);
        let second = vec![0i16; RATE as usize];
        for seq in 0..MAX_SECONDS {
            long.receive(chunk(1, seq, &second), RATE);
        }
        assert_eq!(long.pending().len(), (MAX_SECONDS * RATE) as usize);
        long.receive(chunk(1, MAX_SECONDS, &[0]), RATE);
        assert!(!long.playing());
    }

    #[test]
    fn stop_from_the_desktop_or_the_user_silences_it() {
        let mut arriving = player([start(1), chunk(1, 0, &[1, 2])]);
        arriving.receive(Speech::Stop { reply: 1 }, RATE);
        assert!(!arriving.playing());
        // Ended, still playing.
        let mut ended = player_ended();
        ended.receive(Speech::Stop { reply: 1 }, RATE);
        assert!(!ended.playing());
        let mut talking = player_ended();
        talking.stop();
        assert!(!talking.playing() && talking.pending().is_empty());
        // A stop for the reply before leaves the one arriving.
        let mut newer = player([start(2), chunk(2, 0, &[1])]);
        newer.receive(Speech::Stop { reply: 1 }, RATE);
        assert_eq!(newer.pending(), [1]);
    }

    #[test]
    fn a_new_start_replaces_the_reply_playing() {
        let player = player([start(1), chunk(1, 0, &[1, 2]), start(2), chunk(2, 0, &[7])]);
        assert_eq!(player.pending(), [7]);
    }

    #[test]
    fn audio_is_resampled_to_the_outputs_rate() {
        let mut player = Player::default();
        player.receive(start(1), 48_000);
        player.receive(chunk(1, 0, &[10, 20]), 48_000);
        assert_eq!(player.pending(), [10, 10, 10, 20, 20, 20]);
    }
}
