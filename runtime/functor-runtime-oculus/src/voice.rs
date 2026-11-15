//! Push-to-talk: hold the right thumbstick click
//! ([`functor_runtime_common::link_overlay::PushToTalk`]), speak, release.
//! The microphone is open only while the button is held and the link is up;
//! its audio goes straight onto the link ([`functor_device_link::talk`]),
//! and nothing is written to storage. The desktop's progress comes back on
//! the toast.
//!
//! The microphone is AAudio, read without blocking once a frame: the APK
//! plays no sound yet, so there is no audio stack to share, and AAudio is
//! the NDK's own. The first hold asks for `RECORD_AUDIO`; the toast says
//! when it was allowed (no restart needed) or how to allow it.

use std::time::{Duration, Instant};

use functor_device_link::client::Client;
use functor_device_link::talk::{Stopped, Talk};
use functor_device_link::voice::{Capsule, Pose, Stage, Status, MAX_SECONDS, STATUS_CHANNEL};
use functor_runtime_common::debug_protocol::{VoiceStage, VoiceToast};
use functor_runtime_common::link_overlay::{self, Follow, Progress, PushToTalk, Toast};
use functor_runtime_common::{TrackingPose, XrInputSnapshot};
use jni::objects::JObject;
use jni::{jni_sig, jni_str, JValue};
use ndk_sys as ffi;

/// The countdown shows for the last seconds before it sends by itself.
const COUNTDOWN: u64 = 5;
const LIMIT: Duration = Duration::from_secs(MAX_SECONDS as u64);
const PERMISSION: &str = "android.permission.RECORD_AUDIO";
/// After asking: how often, and how long, the answer is looked for.
const PERMISSION_POLL: Duration = Duration::from_millis(500);
const PERMISSION_WAIT: Duration = Duration::from_secs(120);

pub struct Voice {
    gesture: PushToTalk,
    talk: Talk,
    mic: Option<Mic>,
    /// The microphone prompt was shown at this time, and its answer is
    /// looked for at `next_check`; `asked` stays once asked.
    prompted: Option<(Instant, Instant)>,
    asked: bool,
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
            prompted: None,
            asked: false,
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
    /// One frame: `received` is the link's channel messages since the last
    /// (statuses are taken from it), `sample` drives the gesture, `armed`
    /// while it may talk; `capsule` (JSON) is made only when an utterance
    /// starts.
    pub fn frame(
        &mut self,
        client: &Client,
        received: &[(u16, Vec<u8>)],
        sample: Option<&XrInputSnapshot>,
        armed: bool,
        capsule: impl FnOnce() -> String,
        now: Instant,
    ) {
        for (channel, data) in received {
            if *channel == STATUS_CHANNEL {
                match Status::decode(data) {
                    Ok(status) => self.status(status, now),
                    Err(e) => log::warn!("voice: a status we cannot read: {e}"),
                }
            }
        }
        self.permission_answered(now);
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

    /// The right hand is the gesture's: the panel leaves it alone.
    pub fn busy(&self) -> bool {
        self.gesture.busy()
    }

    /// Recording: what is said shows in the panel's Talk section.
    pub fn listening(&self) -> bool {
        self.talk.utterance().is_some()
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
        let mic = match Mic::tone().map(Ok).unwrap_or_else(|| self.microphone(now)) {
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

    /// The microphone, or the stage that says why not: the system's prompt
    /// the first time, how to allow it in Settings after (Android shows the
    /// prompt only so often).
    fn microphone(&mut self, now: Instant) -> Result<Mic, VoiceStage> {
        let failed = |reason: String| VoiceStage::NotSent { reason };
        match permission_granted() {
            Ok(true) => {
                self.prompted = None;
                Mic::open().map_err(|e| {
                    log::warn!("voice: {e}");
                    failed(e)
                })
            }
            Ok(false) if self.asked => Err(VoiceStage::MicrophoneDenied),
            Ok(false) => {
                if let Err(e) = request_permission() {
                    log::warn!("voice: asking for the microphone: {e}");
                }
                self.asked = true;
                self.prompted = Some((now, now + PERMISSION_POLL));
                Err(VoiceStage::AllowMicrophone)
            }
            Err(e) => Err(failed(format!("Microphone permission: {e}"))),
        }
    }

    /// After the prompt: once the microphone is allowed, says so.
    fn permission_answered(&mut self, now: Instant) {
        let Some((since, next)) = self.prompted else {
            return;
        };
        if now < next {
            return;
        }
        if permission_granted().unwrap_or(false) {
            self.prompted = None;
            self.toast.show(VoiceStage::MicrophoneAllowed, now);
        } else if now.duration_since(since) >= PERMISSION_WAIT {
            self.prompted = None;
        } else {
            self.prompted = Some((since, now + PERMISSION_POLL));
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

    /// The desktop's progress, queued on the toast. Ignored while the user
    /// is talking, and for an utterance older than the last (one from
    /// before this app started still shows).
    fn status(&mut self, status: Status, now: Instant) {
        let stale = status
            .utterance
            .is_some_and(|u| self.last.is_some_and(|last| u < last));
        let shown = !stale && self.talk.utterance().is_none();
        log::info!(
            "voice: the desktop says {:?} for {:?}{}",
            status.stage,
            status.utterance,
            if shown { "" } else { " (not shown)" }
        );
        if !shown {
            return;
        }
        let step = match status.stage {
            Stage::Heard => Progress::Stage(VoiceStage::Heard),
            Stage::Transcribed { text } => Progress::Transcript(text),
            Stage::Working => Progress::Stage(VoiceStage::Working),
            Stage::Reloaded => Progress::Stage(VoiceStage::Reloaded),
            Stage::Failed { reason } => Progress::Stage(VoiceStage::Failed { reason }),
            Stage::Idle => Progress::Idle,
            Stage::Answered { text } => Progress::Stage(VoiceStage::Answered { text }),
            Stage::Done { text } => Progress::Stage(VoiceStage::Done { text }),
        };
        self.toast.progress(step, now);
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
        input: Input,
        channels: usize,
        buffer: Vec<i16>,
        /// Since opening: for the log when it closes.
        opened: Instant,
        first: Option<Duration>,
        frames: usize,
        peak: u16,
    },
    /// `debug.functor.voice_tone`: a 440 Hz tone instead of the microphone,
    /// for on-device tests nobody is speaking in. Debug builds only; a
    /// system property, so nothing over the link can turn it on.
    Tone { since: Instant, taken: u64 },
}

impl Mic {
    /// The test tone, when set.
    fn tone() -> Option<Mic> {
        (cfg!(debug_assertions)
            && crate::system_property("debug.functor.voice_tone").as_deref() == Some("1"))
        .then(|| Mic {
            rate: functor_device_link::voice::SAMPLE_RATE,
            source: Source::Tone {
                since: Instant::now(),
                taken: 0,
            },
        })
    }

    /// The microphone, recording; `Err` names what failed.
    fn open() -> Result<Mic, String> {
        let input = Input::open()?;
        let rate = input.rate();
        let channels = input.channels();
        Ok(Mic {
            rate,
            source: Source::Aaudio {
                // A second of audio: more than one frame ever waits.
                buffer: vec![0; rate as usize * channels],
                input,
                channels,
                opened: Instant::now(),
                first: None,
                frames: 0,
                peak: 0,
            },
        })
    }

    /// What it recorded since the last call, first channel only, into `out`.
    fn take(&mut self, out: &mut Vec<i16>) -> Result<(), String> {
        out.clear();
        match &mut self.source {
            Source::Aaudio {
                input,
                channels,
                buffer,
                opened,
                first,
                frames,
                peak,
            } => {
                let read = input.read(buffer, *channels)?;
                out.extend(buffer[..read * *channels].iter().step_by(*channels));
                if read > 0 {
                    first.get_or_insert_with(|| opened.elapsed());
                }
                *frames += read;
                *peak = out.iter().map(|s| s.unsigned_abs()).fold(*peak, u16::max);
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

impl Drop for Mic {
    fn drop(&mut self) {
        if let Source::Aaudio {
            opened,
            first,
            frames,
            peak,
            ..
        } = &self.source
        {
            log::info!(
                "voice: microphone open {:.2} s, first audio after {first:?}, {:.2} s of audio at {} Hz, peak {peak}",
                opened.elapsed().as_secs_f32(),
                *frames as f32 / self.rate as f32,
                self.rate
            );
        }
    }
}

/// An AAudio input stream (16-bit, mono, 16 kHz asked for; the device may
/// give another rate), started; closed on drop. Straight on the NDK: the
/// `ndk` crate's `AudioStream::read` takes any non-zero result, a frame
/// count included, for an error.
struct Input(*mut ffi::AAudioStream);

impl Input {
    fn open() -> Result<Input, String> {
        // SAFETY: the builder is used only between its creation and delete;
        // the stream is owned by `Input` once opened.
        unsafe {
            let mut builder = std::ptr::null_mut();
            check("create", ffi::AAudio_createStreamBuilder(&mut builder))?;
            ffi::AAudioStreamBuilder_setDirection(builder, ffi::AAUDIO_DIRECTION_INPUT as _);
            ffi::AAudioStreamBuilder_setFormat(builder, ffi::AAUDIO_FORMAT_PCM_I16 as _);
            ffi::AAudioStreamBuilder_setChannelCount(builder, 1);
            ffi::AAudioStreamBuilder_setSampleRate(
                builder,
                functor_device_link::voice::SAMPLE_RATE as i32,
            );
            ffi::AAudioStreamBuilder_setInputPreset(
                builder,
                ffi::AAUDIO_INPUT_PRESET_VOICE_RECOGNITION as _,
            );
            ffi::AAudioStreamBuilder_setSharingMode(builder, ffi::AAUDIO_SHARING_MODE_SHARED as _);
            // About a second of room at any rate the device picks (AAudio
            // clamps it): a frame that stalls loses no audio (the default
            // holds about 0.3 s).
            ffi::AAudioStreamBuilder_setBufferCapacityInFrames(builder, 48_000);
            let mut stream = std::ptr::null_mut();
            let opened = ffi::AAudioStreamBuilder_openStream(builder, &mut stream);
            ffi::AAudioStreamBuilder_delete(builder);
            check("open", opened)?;
            let input = Input(stream);
            check("start", ffi::AAudioStream_requestStart(stream))?;
            Ok(input)
        }
    }

    fn rate(&self) -> u32 {
        // SAFETY: an open stream.
        unsafe { ffi::AAudioStream_getSampleRate(self.0) }.max(1) as u32
    }

    fn channels(&self) -> usize {
        // SAFETY: an open stream.
        unsafe { ffi::AAudioStream_getChannelCount(self.0) }.max(1) as usize
    }

    /// Reads what is waiting into `buffer` without blocking: the frames read.
    fn read(&mut self, buffer: &mut [i16], channels: usize) -> Result<usize, String> {
        let frames = (buffer.len() / channels) as i32;
        // SAFETY: `buffer` holds `frames * channels` samples.
        let read =
            unsafe { ffi::AAudioStream_read(self.0, buffer.as_mut_ptr().cast(), frames, 0) };
        check("read", read.min(0))?;
        Ok(read as usize)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: opened by `Input::open`, closed once.
        unsafe { ffi::AAudioStream_close(self.0) };
    }
}

/// An AAudio result: `Ok` for `AAUDIO_OK`, else what failed and its name
/// (`Microphone read: AAUDIO_ERROR_DISCONNECTED`).
fn check(what: &str, result: ffi::aaudio_result_t) -> Result<(), String> {
    if result == ffi::AAUDIO_OK as ffi::aaudio_result_t {
        return Ok(());
    }
    // SAFETY: AAudio returns a static string for any result.
    let name = unsafe { std::ffi::CStr::from_ptr(ffi::AAudio_convertResultToText(result)) };
    Err(format!("Microphone {what}: {}", name.to_string_lossy()))
}

/// Whether the app holds `RECORD_AUDIO`.
fn permission_granted() -> jni::errors::Result<bool> {
    with_activity(|env, activity| {
        let name = env.new_string(PERMISSION)?;
        Ok(env
            .call_method(
                activity,
                jni_str!("checkSelfPermission"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[JValue::Object(&name)],
            )?
            .i()?
            == 0)
    })
}

/// Shows the system's microphone prompt (if the user has not turned it off
/// for good); the answer is read back with [`permission_granted`].
fn request_permission() -> jni::errors::Result<()> {
    with_activity(|env, activity| {
        let name = env.new_string(PERMISSION)?;
        let names = env.new_object_array(1, jni_str!("java/lang/String"), &name)?;
        env.call_method(
            activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::Object(&names), JValue::Int(1)],
        )?;
        Ok(())
    })
}

fn with_activity<T>(
    f: impl FnOnce(&mut jni::Env, &JObject) -> jni::errors::Result<T>,
) -> jni::errors::Result<T> {
    let context = ndk_context::android_context();
    // SAFETY: the VM and the activity outlive the app's frames.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
    vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(env, context.context().cast()) };
        f(env, &activity)
    })
}
