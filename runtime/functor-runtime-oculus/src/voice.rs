//! Push-to-talk: hold the right thumbstick click
//! ([`functor_runtime_common::link_overlay::PushToTalk`]), speak, release.
//! The microphone is open only while the button is held and the link is up;
//! its audio goes straight onto the link ([`functor_device_link::talk`]),
//! and nothing is written to storage. The desktop's progress comes back on
//! the toast.
//!
//! The microphone is AAudio, read without blocking once a frame: the APK
//! plays no sound yet, so there is no audio stack to share, and AAudio is
//! the NDK's own (the `ndk` crate android-activity already links).

use std::time::{Duration, Instant};

use functor_device_link::client::Client;
use functor_device_link::talk::{Stopped, Talk};
use functor_device_link::voice::{Capsule, Pose, Stage, Status, MAX_SECONDS, STATUS_CHANNEL};
use functor_runtime_common::debug_protocol::{VoiceStage, VoiceToast};
use functor_runtime_common::link_overlay::{self, Follow, PushToTalk, Toast};
use functor_runtime_common::{TrackingPose, XrInputSnapshot};
use jni::objects::JObject;
use jni::{jni_sig, jni_str, JValue};
use ndk::audio::{
    AudioDirection, AudioFormat, AudioInputPreset, AudioSharingMode, AudioStream,
    AudioStreamBuilder,
};

/// The countdown shows for the last seconds before it sends by itself.
const COUNTDOWN: u64 = 5;
const LIMIT: Duration = Duration::from_secs(MAX_SECONDS as u64);
const PERMISSION: &str = "android.permission.RECORD_AUDIO";

pub struct Voice {
    gesture: PushToTalk,
    talk: Talk,
    mic: Option<Mic>,
    /// The link the utterance is on ([`Client::link`]).
    link: u64,
    started: Instant,
    /// The utterance the desktop's statuses are about.
    last: Option<u64>,
    toast: Toast,
    pub follow: Follow,
    samples: Vec<i16>,
}

impl Default for Voice {
    fn default() -> Voice {
        // Ids unique across restarts: start from the clock.
        let first = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as u64);
        Voice {
            gesture: PushToTalk::default(),
            talk: Talk::new(first),
            mic: None,
            link: 0,
            started: Instant::now(),
            last: None,
            toast: Toast::default(),
            follow: Follow::default(),
            samples: Vec::new(),
        }
    }
}

impl Voice {
    /// One frame: `sample` drives the gesture, `armed` while it may talk;
    /// `capsule` (JSON) is made only when an utterance starts.
    pub fn frame(
        &mut self,
        client: &Client,
        sample: Option<&XrInputSnapshot>,
        armed: bool,
        capsule: impl FnOnce() -> String,
        now: Instant,
    ) {
        for (channel, data) in client.received() {
            if channel == STATUS_CHANNEL {
                match Status::decode(&data) {
                    Ok(status) => self.status(status, now),
                    Err(e) => log::warn!("voice: a status we cannot read: {e}"),
                }
            }
        }
        match self.gesture.step(sample, armed, now) {
            Some(link_overlay::Talk::Start) => self.start(client, capsule, now),
            Some(link_overlay::Talk::Send) => self.send(client, now),
            Some(link_overlay::Talk::Cancel) => self.cancel(client, "Cancelled", now),
            None => {}
        }
        if self.talk.utterance().is_none() {
            return;
        }
        // Gone, or gone and back: this utterance was on the last link.
        if client.link() != Some(self.link) {
            let stopped = self.talk.lost();
            return self.stopped(stopped, now);
        }
        if let Err(e) = self.listen(client, now) {
            return self.cancel(client, &e, now);
        }
        if self.talk.utterance().is_none() {
            return;
        }
        let held = now.duration_since(self.started);
        if held >= LIMIT {
            return self.send(client, now);
        }
        let left = (LIMIT - held).as_secs_f32().ceil() as u64;
        self.toast.show(
            VoiceStage::Listening {
                level: (self.talk.level() * link_overlay::METER as f32).round() as u8,
                seconds_left: (left <= COUNTDOWN).then_some(left as u32),
            },
            now,
        );
    }

    /// No frames for a while (the session stopped, tracking lost): an
    /// utterance in progress is cancelled rather than left recording.
    pub fn pause(&mut self, client: &Client, now: Instant) {
        if self.talk.utterance().is_some() {
            self.cancel(client, "The headset paused", now);
        }
    }

    /// The game's view of the right hand while the gesture holds it.
    pub fn game_view(&mut self, xr: &mut XrInputSnapshot, stepping: bool) {
        self.gesture.game_view(xr, stepping);
    }

    pub fn toast(&mut self, now: Instant) -> Option<(&VoiceToast, f32)> {
        let look = self.toast.look(now);
        if look.is_none() {
            self.follow.reset();
        }
        look
    }

    fn start(&mut self, client: &Client, capsule: impl FnOnce() -> String, now: Instant) {
        let Some(link) = client.link() else {
            self.gesture.stop();
            return self.toast.show(VoiceStage::NotConnected, now);
        };
        let mic = match Mic::open() {
            Ok(mic) => mic,
            Err(stage) => {
                self.gesture.stop();
                return self.toast.show(stage, now);
            }
        };
        match self.talk.start(mic.rate, capsule(), &mut (client, link)) {
            Some(id) => {
                log::info!("voice: utterance {id} at {} Hz", mic.rate);
                self.mic = Some(mic);
                self.link = link;
                self.last = Some(id);
                self.started = now;
                self.toast.show(
                    VoiceStage::Listening {
                        level: 0,
                        seconds_left: None,
                    },
                    now,
                );
            }
            None => {
                self.gesture.stop();
                self.toast.show(VoiceStage::NotConnected, now);
            }
        }
    }

    /// What the microphone recorded since the last frame, onto the link.
    fn listen(&mut self, client: &Client, now: Instant) -> Result<(), String> {
        if let Some(mic) = &mut self.mic {
            mic.take(&mut self.samples)?;
        }
        if let Some(stopped) = self.talk.push(&self.samples, &mut (client, self.link)) {
            self.stopped(Some(stopped), now);
        }
        Ok(())
    }

    /// Released, or at the limit: the rest of the audio, then `End`.
    fn send(&mut self, client: &Client, now: Instant) {
        if let Err(e) = self.listen(client, now) {
            return self.cancel(client, &e, now);
        }
        let stopped = self.talk.finish(&mut (client, self.link));
        self.stopped(stopped, now);
    }

    fn cancel(&mut self, client: &Client, reason: &str, now: Instant) {
        if self.talk.cancel(&mut (client, self.link)).is_some() {
            log::info!("voice: cancelled: {reason}");
            self.stopped(None, now);
            self.toast.show(
                VoiceStage::NotSent {
                    reason: reason.to_string(),
                },
                now,
            );
        }
    }

    fn stopped(&mut self, stopped: Option<Stopped>, now: Instant) {
        self.mic = None;
        self.gesture.stop();
        let reason = |r: &str| VoiceStage::NotSent {
            reason: r.to_string(),
        };
        let stage = match stopped {
            None => return,
            Some(Stopped::Sent) => VoiceStage::Sending,
            Some(Stopped::Cancelled) => reason("Cancelled"),
            Some(Stopped::TooSlow) => reason("The link is too slow"),
            Some(Stopped::Lost) => reason("The desktop link dropped"),
        };
        log::info!("voice: {stopped:?}");
        self.toast.show(stage, now);
    }

    /// The desktop's progress. Ignored while the user is talking, and for an
    /// earlier utterance than the last.
    fn status(&mut self, status: Status, now: Instant) {
        if self.talk.utterance().is_some() || status.utterance.is_some_and(|u| Some(u) != self.last)
        {
            return;
        }
        let stage = match status.stage {
            Stage::Heard => VoiceStage::Heard,
            Stage::Transcribed { text } => {
                if self.toast.stage().is_none() {
                    self.toast.show(VoiceStage::Heard, now);
                }
                return self.toast.transcript(text, now);
            }
            Stage::Working => VoiceStage::Working,
            Stage::Reloaded => VoiceStage::Reloaded,
            Stage::Failed { reason } => VoiceStage::Failed { reason },
            Stage::Idle => return self.toast.hide(now),
        };
        self.toast.show(stage, now);
    }
}

/// What the user was looking at as they started speaking: the frame (as
/// `/state` numbers it), game time, and head and grip poses (as
/// `/state.input.xr` reports them).
pub fn capsule(frame: u64, tts: f64, xr: Option<&XrInputSnapshot>) -> String {
    let pose = |p: Option<TrackingPose>| {
        p.map(|p| Pose {
            position: p.position,
            orientation: p.orientation,
        })
    };
    Capsule {
        frame: Some(frame),
        tts: Some(tts),
        head: pose(xr.and_then(|x| x.head)),
        left: pose(xr.and_then(|x| x.left.grip)),
        right: pose(xr.and_then(|x| x.right.grip)),
        ..Capsule::default()
    }
    .to_json()
}

/// The open microphone. Closed (and stopped) on drop.
struct Mic {
    rate: u32,
    source: Source,
}

enum Source {
    Aaudio {
        stream: AudioStream,
        channels: usize,
        buffer: Vec<i16>,
    },
    /// `debug.functor.voice_tone`: a 440 Hz tone instead of the microphone,
    /// for on-device tests nobody is speaking in. Debug builds only; a
    /// system property, so nothing over the link can turn it on.
    Tone { since: Instant, taken: u64 },
}

impl Mic {
    /// Opens the microphone, or says why not as the toast's stage.
    fn open() -> Result<Mic, VoiceStage> {
        if cfg!(debug_assertions)
            && crate::system_property("debug.functor.voice_tone").as_deref() == Some("1")
        {
            return Ok(Mic {
                rate: functor_device_link::voice::SAMPLE_RATE,
                source: Source::Tone {
                    since: Instant::now(),
                    taken: 0,
                },
            });
        }
        let failed = |e: String| VoiceStage::NotSent {
            reason: format!("Microphone: {e}"),
        };
        match permission() {
            Ok(true) => {}
            Ok(false) => return Err(VoiceStage::AllowMicrophone),
            Err(e) => return Err(failed(e.to_string())),
        }
        let stream = AudioStreamBuilder::new()
            .and_then(|b| {
                b.direction(AudioDirection::Input)
                    .format(AudioFormat::PCM_I16)
                    .channel_count(1)
                    .sample_rate(functor_device_link::voice::SAMPLE_RATE as i32)
                    .input_preset(AudioInputPreset::VoiceRecognition)
                    .sharing_mode(AudioSharingMode::Shared)
                    .open_stream()
            })
            .map_err(|e| failed(format!("{e:?}")))?;
        stream
            .request_start()
            .map_err(|e| failed(format!("{e:?}")))?;
        let rate = stream.sample_rate().max(1) as u32;
        let channels = stream.channel_count().max(1) as usize;
        Ok(Mic {
            rate,
            source: Source::Aaudio {
                // A second of audio: more than one frame ever waits.
                buffer: vec![0; rate as usize * channels],
                stream,
                channels,
            },
        })
    }

    /// What it recorded since the last call, first channel only, into `out`.
    fn take(&mut self, out: &mut Vec<i16>) -> Result<(), String> {
        out.clear();
        match &mut self.source {
            Source::Aaudio {
                stream,
                channels,
                buffer,
            } => {
                let frames = (buffer.len() / *channels) as i32;
                // SAFETY: `buffer` holds `frames * channels` samples.
                let read = unsafe { stream.read(buffer.as_mut_ptr().cast(), frames, 0) }
                    .map_err(|e| format!("Microphone: {e:?}"))?;
                out.extend(
                    buffer[..read as usize * *channels]
                        .iter()
                        .step_by(*channels),
                );
            }
            Source::Tone { since, taken } => {
                let due = (since.elapsed().as_secs_f64() * self.rate as f64) as u64;
                out.extend((*taken..due).map(|i| {
                    let t = i as f32 / self.rate as f32;
                    (8000.0 * (t * 440.0 * std::f32::consts::TAU).sin()) as i16
                }));
                *taken = due;
            }
        }
        Ok(())
    }
}

/// Whether the app holds `RECORD_AUDIO`; asks for it when not (the system
/// shows its prompt, and the user holds the button again after).
fn permission() -> jni::errors::Result<bool> {
    let context = ndk_context::android_context();
    // SAFETY: the VM and the activity outlive the app's frames.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
    vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let activity = unsafe { JObject::from_raw(env, context.context().cast()) };
        let name = env.new_string(PERMISSION)?;
        let granted = env
            .call_method(
                &activity,
                jni_str!("checkSelfPermission"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[JValue::Object(&name)],
            )?
            .i()?
            == 0;
        if !granted {
            let names = env.new_object_array(1, jni_str!("java/lang/String"), &name)?;
            env.call_method(
                &activity,
                jni_str!("requestPermissions"),
                jni_sig!("([Ljava/lang/String;I)V"),
                &[JValue::Object(&names), JValue::Int(1)],
            )?;
        }
        Ok(granted)
    })
}
