//! Spoken replies, the shell's half (D47): the core's text turned into
//! sound here on this machine and played on its speakers. One reply at a
//! time: a new one replaces one still playing, and [`Replies::stop`] (the
//! user started talking) silences it. Off with `FUNCTOR_APP_SPEAK=off`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Turns text into speech: 16-bit mono PCM at [`functor_stt::SAMPLE_RATE`],
/// the link's audio format.
pub trait Synth: Send + Sync {
    fn synth(&self, text: &str) -> Result<Vec<i16>, String>;
}

/// Plays speech; dropping what it returns stops it.
pub trait Output: Send + Sync {
    fn play(&self, pcm: Vec<i16>) -> Result<Box<dyn Send>, String>;
}

pub struct Replies {
    synth: Box<dyn Synth>,
    output: Box<dyn Output>,
    /// The reply asked for last (its number), and what is playing.
    now: Mutex<(u64, Option<Box<dyn Send>>)>,
}

impl Replies {
    pub fn new(synth: Box<dyn Synth>, output: Box<dyn Output>) -> Arc<Replies> {
        Arc::new(Replies {
            synth,
            output,
            now: Mutex::default(),
        })
    }

    /// The system voice and speakers, where there are any (macOS), unless
    /// `FUNCTOR_APP_SPEAK=off`. With `FUNCTOR_APP_FAKE_VOICE=<file>` each
    /// reply and stop is written there instead (tests); a fake agent or
    /// transcript without it is a test too, and makes no sound.
    pub fn from_env() -> Option<Arc<Replies>> {
        if std::env::var_os("FUNCTOR_APP_SPEAK").is_some_and(|v| v == "off") {
            return None;
        }
        if let Some(log) = std::env::var_os("FUNCTOR_APP_FAKE_VOICE") {
            let log = PathBuf::from(log);
            return Some(Replies::new(Box::new(Fake), Box::new(Log(log))));
        }
        let faked = ["FUNCTOR_APP_FAKE_AGENT", "FUNCTOR_APP_FAKE_TRANSCRIPT"];
        if faked.iter().any(|var| std::env::var_os(var).is_some()) {
            return None;
        }
        #[cfg(target_os = "macos")]
        let real = Some(Replies::new(Box::new(mac::Say), Box::new(mac::Afplay)));
        #[cfg(not(target_os = "macos"))]
        let real = None;
        real
    }

    /// A new reply, in place of whatever is playing: its number, for
    /// [`Replies::say`]. Taken where replies are asked for, so the last
    /// asked for is the one heard.
    pub fn next(&self) -> u64 {
        let mut now = self.now.lock().unwrap();
        *now = (now.0 + 1, None);
        now.0
    }

    /// Says reply `mine`, unless stopped or replaced meanwhile: how long
    /// it plays for. Blocks while it is synthesised: call it off the async
    /// threads.
    pub fn say(&self, mine: u64, text: &str) -> Result<std::time::Duration, String> {
        let pcm = self.synth.synth(text)?;
        let seconds = pcm.len() as f32 / functor_stt::SAMPLE_RATE as f32;
        let mut now = self.now.lock().unwrap();
        // Stopped, or replaced, while it was synthesised.
        if now.0 == mine {
            now.1 = Some(self.output.play(pcm)?);
        }
        Ok(std::time::Duration::from_secs_f32(seconds))
    }

    /// `functor-app say`: says `text` as a reply is said and waits until
    /// it has played, to hear the voice without a headset or an agent.
    pub fn hear(text: &str) -> Result<(), String> {
        let replies = Replies::from_env().ok_or(
            "there is no voice here: replies are spoken on macOS, unless FUNCTOR_APP_SPEAK=off",
        )?;
        let length = replies.say(replies.next(), text)?;
        std::thread::sleep(length + std::time::Duration::from_millis(300));
        Ok(())
    }

    /// Silences the reply playing, and one still being synthesised.
    pub fn stop(&self) {
        let mut now = self.now.lock().unwrap();
        *now = (now.0 + 1, None);
    }
}

/// A sample per character: silence of a known length.
struct Fake;

impl Synth for Fake {
    fn synth(&self, text: &str) -> Result<Vec<i16>, String> {
        Ok(vec![0; text.chars().count()])
    }
}

/// Writes `play <samples>` and `stop` lines to a file.
struct Log(PathBuf);

impl Log {
    fn line(path: &std::path::Path, line: &str) {
        use std::io::Write;
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path);
        if let Ok(mut file) = file {
            let _ = writeln!(file, "{line}");
        }
    }
}

struct Logged(PathBuf);

impl Drop for Logged {
    fn drop(&mut self) {
        Log::line(&self.0, "stop");
    }
}

impl Output for Log {
    fn play(&self, pcm: Vec<i16>) -> Result<Box<dyn Send>, String> {
        Log::line(&self.0, &format!("play {}", pcm.len()));
        Ok(Box::new(Logged(self.0.clone())))
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};

    use super::{Output, Synth};

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("functor-reply-{}.wav", uuid::Uuid::new_v4()))
    }

    /// The system voice (`say`), to a WAV it then reads: nothing to
    /// download, and nothing leaves the machine.
    pub struct Say;

    impl Synth for Say {
        fn synth(&self, text: &str) -> Result<Vec<i16>, String> {
            let path = temp();
            let format = format!("--data-format=LEI16@{}", functor_stt::SAMPLE_RATE);
            // After `--`, so none of the text is read as an option.
            let run = || {
                Command::new("/usr/bin/say")
                    .arg("-o")
                    .arg(&path)
                    .args([format.as_str(), "--", text])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
            };
            let result = match run() {
                Ok(status) if status.success() => std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|wav| functor_stt::read_wav(&wav)),
                Ok(status) => Err(format!("say ended with {status}")),
                Err(e) => Err(format!("cannot run say: {e}")),
            };
            let _ = std::fs::remove_file(&path);
            result
        }
    }

    /// The default output device, through `afplay`.
    pub struct Afplay;

    /// Killed, and its file removed, when dropped.
    struct Playing(Child, PathBuf);

    impl Drop for Playing {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
            let _ = std::fs::remove_file(&self.1);
        }
    }

    impl Output for Afplay {
        fn play(&self, pcm: Vec<i16>) -> Result<Box<dyn Send>, String> {
            let path = temp();
            std::fs::write(&path, functor_stt::write_wav(&pcm)).map_err(|e| e.to_string())?;
            let child = Command::new("/usr/bin/afplay")
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match child {
                Ok(child) => Ok(Box::new(Playing(child, path))),
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    Err(format!("cannot run afplay: {e}"))
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The real system voice: a sentence comes back as seconds of sound.
        /// Not on CI, whose runners have no voice: `cargo test -- --ignored`.
        #[test]
        #[ignore = "needs a system voice"]
        fn the_system_voice_speaks_a_sentence() {
            let pcm = Say.synth("The ball is red now.").unwrap();
            let seconds = pcm.len() as f32 / functor_stt::SAMPLE_RATE as f32;
            assert!((0.5..10.0).contains(&seconds), "{seconds} s");
            assert!(pcm.iter().any(|s| s.unsigned_abs() > 500), "not silence");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// What the fake output saw, in order.
    #[derive(Debug, PartialEq)]
    enum Seen {
        Play(usize),
        Stop,
    }

    struct Out(mpsc::Sender<Seen>);
    struct Playing(mpsc::Sender<Seen>);

    impl Drop for Playing {
        fn drop(&mut self) {
            let _ = self.0.send(Seen::Stop);
        }
    }

    impl Output for Out {
        fn play(&self, pcm: Vec<i16>) -> Result<Box<dyn Send>, String> {
            let _ = self.0.send(Seen::Play(pcm.len()));
            Ok(Box::new(Playing(self.0.clone())))
        }
    }

    /// Says it has started, then synthesises when told to, so a test can
    /// act meanwhile.
    struct Slow(Mutex<(mpsc::Sender<()>, mpsc::Receiver<()>)>);

    impl Synth for Slow {
        fn synth(&self, text: &str) -> Result<Vec<i16>, String> {
            let (started, go) = &*self.0.lock().unwrap();
            started.send(()).map_err(|e| e.to_string())?;
            go.recv().map_err(|e| e.to_string())?;
            Fake.synth(text)
        }
    }

    #[test]
    fn a_reply_plays_a_new_one_replaces_it_and_talking_stops_it() {
        let (tx, seen) = mpsc::channel();
        let replies = Replies::new(Box::new(Fake), Box::new(Out(tx)));
        replies.say(replies.next(), "one").unwrap();
        assert_eq!(seen.try_recv(), Ok(Seen::Play(3)));
        // A new reply: the first stops, then the second plays.
        replies.say(replies.next(), "three").unwrap();
        assert_eq!(seen.try_iter().collect::<Vec<_>>(), [Seen::Stop, Seen::Play(5)]);
        // Asked for in order, synthesised out of order: the last asked for.
        let (older, newer) = (replies.next(), replies.next());
        replies.say(newer, "newer").unwrap();
        replies.say(older, "old").unwrap();
        assert_eq!(seen.try_iter().collect::<Vec<_>>(), [Seen::Stop, Seen::Play(5)]);
        // The user starts talking.
        replies.stop();
        assert_eq!(seen.try_iter().collect::<Vec<_>>(), [Seen::Stop]);
        replies.stop();
        assert!(seen.try_recv().is_err(), "nothing left to stop");
    }

    #[test]
    fn a_reply_stopped_while_synthesised_never_plays() {
        let (tx, seen) = mpsc::channel();
        let (go, wait) = mpsc::channel();
        let (started, synthesising) = mpsc::channel();
        let slow = Slow(Mutex::new((started, wait)));
        let replies = Replies::new(Box::new(slow), Box::new(Out(tx)));
        let saying = {
            let replies = replies.clone();
            let mine = replies.next();
            std::thread::spawn(move || replies.say(mine, "late"))
        };
        // Talking began before it was ready.
        synthesising.recv().unwrap();
        replies.stop();
        go.send(()).unwrap();
        saying.join().unwrap().unwrap();
        assert!(seen.try_recv().is_err());
    }
}
