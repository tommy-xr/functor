//! Push-to-talk on a Quest attached over adb (`npm run test:quest-voice`):
//! with `FUNCTOR_QUEST_E2E=1` and a debuggable `dev.functor.runner`
//! installed, on the same Wi-Fi as this machine. A stub desktop pairs with
//! it by its file and is found by mDNS; held thumbstick presses are injected
//! (`debug.functor.pair_input`) and the microphone is a test tone
//! (`debug.functor.voice_tone`), since nobody wears the headset. The stub
//! checks the utterance and sends statuses the toast must show. The
//! headset's pairing file and properties are put back afterwards; stills
//! land in `FUNCTOR_QUEST_CAPTURES` when set.
#![cfg(feature = "client")]

mod stub;

use stub::until;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use functor_device_link::adb::{self, PACKAGE};
use functor_device_link::client::Stored;
use functor_device_link::voice::{Capsule, Clip, Stage, Status, CLIP_CHANNEL, SAMPLE_RATE};
use functor_device_link::{hex, os_random, Secret, SERVICE};
use serde_json::{json, Value};

const PROPS: [&str; 3] = [
    // Guardian would hold the app at its boundary dialog.
    "debug.oculus.guardian_pause",
    "debug.functor.pair_input",
    "debug.functor.voice_tone",
];

struct Quest {
    serial: String,
    debug: String,
    pairing: Vec<u8>,
    props: Vec<(&'static str, String)>,
}

impl Quest {
    fn attach() -> Quest {
        let devices = Command::new("adb").arg("devices").output().unwrap();
        let serial = adb::devices(&String::from_utf8_lossy(&devices.stdout))
            .into_iter()
            .find(|e| e.state == "device")
            .expect("a Quest attached over adb")
            .serial;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut quest = Quest {
            serial,
            debug: format!("127.0.0.1:{port}"),
            pairing: Vec::new(),
            props: Vec::new(),
        };
        let saved = quest.adb(&["shell", adb::READ_PAIRING]);
        assert!(
            saved.status.success(),
            "a debuggable {PACKAGE} is installed"
        );
        quest.pairing = saved.stdout;
        for name in PROPS {
            let before = quest.shell(&format!("getprop {name}"));
            quest.props.push((name, before));
            quest.shell(&format!("setprop {name} 1"));
        }
        quest.adb(&["forward", &format!("tcp:{port}"), "tcp:8123"]);
        quest
    }

    fn adb(&self, args: &[&str]) -> Output {
        Command::new("adb")
            .args(["-s", &self.serial])
            .args(args)
            .output()
            .unwrap()
    }

    fn shell(&self, command: &str) -> String {
        let out = self.adb(&["shell", command]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn put_pairing(&self, bytes: &[u8]) {
        let mut child = Command::new("adb")
            .args(["-s", &self.serial, "shell", adb::WRITE_PAIRING])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        assert!(child.wait().unwrap().success());
    }

    fn launch(&self) {
        self.shell("am broadcast -a com.oculus.vrpowermanager.prox_close");
        self.shell(&format!(
            "am start -S -n {PACKAGE}/android.app.NativeActivity"
        ));
        let rendered = || self.state().ok()?["rendered_frames"].as_u64();
        let first = until("the app answers", rendered);
        until("it renders", || rendered().filter(|n| *n > first));
    }

    fn state(&self) -> Result<Value, String> {
        let (status, body) = http(&self.debug, "GET", "/state?model=0", b"")?;
        if status != 200 {
            return Err(format!("{status}"));
        }
        serde_json::from_slice(&body).map_err(|e| e.to_string())
    }

    fn voice(&self) -> Value {
        self.state().unwrap()["link"]["voice"].clone()
    }

    /// The right controller with `fields` set.
    fn hold(&self, fields: Value) {
        let mut right = json!({"active": true});
        right
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let sample = json!({"type": "xr", "right": right}).to_string();
        let (status, body) = http(&self.debug, "POST", "/input", sample.as_bytes()).unwrap();
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    }

    fn capture(&self, name: &str) {
        let Some(dir) = std::env::var_os("FUNCTOR_QUEST_CAPTURES") else {
            return;
        };
        let (status, png) = until("a capture", || {
            http(&self.debug, "POST", "/capture", b"")
                .ok()
                .filter(|(s, _)| *s == 200)
        });
        assert_eq!(status, 200);
        std::fs::write(Path::new(&dir).join(name), png).unwrap();
    }
}

impl Drop for Quest {
    fn drop(&mut self) {
        let _ = http(&self.debug, "POST", "/input", br#"{"type":"xr_clear"}"#);
        self.put_pairing(&self.pairing);
        for (name, value) in &self.props {
            let value = if value.is_empty() { "0" } else { value };
            self.shell(&format!("setprop {name} {value}"));
        }
        let port = self.debug.rsplit(':').next().unwrap().to_string();
        let _ = self.adb(&["forward", "--remove", &format!("tcp:{port}")]);
    }
}

/// One HTTP/1.1 exchange: status and body.
fn http(addr: &str, method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(body).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no head")?;
    let status = String::from_utf8_lossy(&bytes[..split])
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("no status")?;
    Ok((status, bytes[split + 4..].to_vec()))
}

/// The stub's clip messages until `End` or `Cancel`.
fn receive(stub: &stub::Stub) -> Vec<Clip> {
    let mut got = Vec::new();
    loop {
        let (_, channel, data) = stub
            .received
            .recv_timeout(Duration::from_secs(20))
            .expect("the utterance arrives");
        assert_eq!(channel, CLIP_CHANNEL);
        let clip = Clip::decode(&data).expect("well formed");
        let done = matches!(clip, Clip::End { .. } | Clip::Cancel { .. });
        got.push(clip);
        if done {
            return got;
        }
    }
}

/// Advertises the stub as desktop `id` on `port`, under a host name of its
/// own (never the machine's).
fn advertise(id: &[u8; 16], port: u16) -> mdns_sd::ServiceDaemon {
    let daemon = mdns_sd::ServiceDaemon::new().unwrap();
    let props: HashMap<String, String> = [
        ("v".to_string(), functor_device_link::VERSION.to_string()),
        ("id".to_string(), hex(id)),
    ]
    .into();
    let host = format!("functor-{}.local.", hex(id));
    let info = mdns_sd::ServiceInfo::new(SERVICE, "Voice Stub", &host, "", port, props)
        .unwrap()
        .enable_addr_auto();
    daemon.register(info).unwrap();
    daemon
}

#[test]
fn quest_talks_to_a_desktop() {
    if std::env::var_os("FUNCTOR_QUEST_E2E").is_none() {
        eprintln!("skipped: set FUNCTOR_QUEST_E2E=1 with a Quest attached");
        return;
    }
    let quest = Quest::attach();
    let (id, secret) = (os_random::<16>(), Secret(os_random::<32>()));
    let stub = stub::start("0.0.0.0:0", id, secret.clone());
    let port = stub.addr.rsplit(':').next().unwrap().parse().unwrap();
    let _mdns = advertise(&id, port);
    // Paired by its file, keeping the headset's own id.
    let mut stored = Stored::parse(&quest.pairing);
    stored.desktop = Some(hex(&id));
    stored.desktop_name = Some("Voice Stub".into());
    stored.secret = Some(secret.to_hex());
    quest.put_pairing(stored.to_json().as_bytes());
    quest.launch();
    until("linked", || {
        (quest.state().ok()?["link"]["linked"] == true).then_some(())
    });

    // A short click is the game's: nothing is recorded.
    quest.hold(json!({"thumbstick_pressed": true}));
    std::thread::sleep(Duration::from_millis(100));
    quest.hold(json!({}));
    std::thread::sleep(Duration::from_millis(500));
    assert!(quest.voice().is_null(), "{}", quest.voice());
    assert!(stub.received.try_recv().is_err());

    // Held: listening, with the tone on the meter.
    let held = Instant::now();
    quest.hold(json!({"thumbstick_pressed": true}));
    let listening = until("listening", || {
        let voice = quest.voice();
        (voice["stage"] == "listening" && voice["level"].as_u64()? > 0).then_some(voice)
    });
    assert!(listening["seconds_left"].is_null());
    quest.capture("voice-listening.png");
    std::thread::sleep(Duration::from_secs(2).saturating_sub(held.elapsed()));
    quest.hold(json!({}));
    let released = held.elapsed();
    until("sending", || {
        (quest.voice()["stage"] == "sending").then_some(())
    });
    quest.capture("voice-sending.png");

    let clips = receive(&stub);
    let Clip::Start {
        utterance, capsule, ..
    } = &clips[0]
    else {
        panic!("starts with Start: {:?}", clips[0]);
    };
    let utterance = *utterance;
    let capsule_json = capsule.clone();
    let capsule = Capsule::from_json(capsule).expect("a capsule");
    assert!(
        capsule.frame.is_some() && capsule.tts.is_some(),
        "{capsule_json}"
    );
    assert_eq!(clips.last(), Some(&Clip::End { utterance }));
    let mut pcm = Vec::new();
    for (seq, clip) in clips[1..clips.len() - 1].iter().enumerate() {
        let Clip::Chunk {
            utterance: u,
            seq: s,
            data,
        } = clip
        else {
            panic!("chunks between: {clip:?}");
        };
        assert_eq!((*u, *s as usize), (utterance, seq));
        pcm.extend(data.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
    }
    // Held about 2 s, the first quarter second the gesture's.
    let seconds = pcm.len() as f64 / SAMPLE_RATE as f64;
    let most = released.as_secs_f64();
    assert!(
        seconds > 1.0 && seconds <= most,
        "{seconds} s of {most} s held"
    );
    // The test tone: 440 Hz, so 880 zero crossings a second.
    let crossings = pcm.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let rate = crossings as f64 / seconds;
    assert!((800.0..960.0).contains(&rate), "{rate} crossings/s");

    // The desktop's progress shows on the toast.
    let say = |stage: Stage| {
        let status = Status {
            utterance: Some(utterance),
            stage,
        };
        stub.send.send(Some(status.frame())).unwrap();
    };
    let shows = |what: &str, stage: &str| {
        until(what, || (quest.voice()["stage"] == stage).then_some(()));
    };
    say(Stage::Heard);
    shows("heard", "heard");
    quest.capture("voice-heard.png");
    say(Stage::Transcribed {
        text: "make the ball red".into(),
    });
    until("the transcript", || {
        (quest.voice()["transcript"] == "make the ball red").then_some(())
    });
    quest.capture("voice-transcribed.png");
    say(Stage::Working);
    shows("working", "working");
    quest.capture("voice-working.png");
    say(Stage::Reloaded);
    shows("reloaded", "reloaded");
    quest.capture("voice-reloaded.png");
    until("it fades", || quest.voice().is_null().then_some(()));

    // B while talking cancels.
    quest.hold(json!({"thumbstick_pressed": true}));
    until("listening again", || {
        (quest.voice()["stage"] == "listening").then_some(())
    });
    quest.hold(json!({"thumbstick_pressed": true, "secondary_pressed": true}));
    until("cancelled", || {
        (quest.voice()["reason"] == "Cancelled").then_some(())
    });
    quest.capture("voice-cancelled.png");
    quest.hold(json!({}));
    let clips = receive(&stub);
    assert!(
        matches!(clips.last(), Some(Clip::Cancel { .. })),
        "{clips:?}"
    );

    // Paired with a desktop that is not there: holding says so.
    let mut away = stored.clone();
    away.desktop = Some(hex(&[0u8; 16]));
    away.secret = Some(Secret(os_random::<32>()).to_hex());
    quest.put_pairing(away.to_json().as_bytes());
    until("unlinked", || {
        (quest.state().ok()?["link"]["linked"] == false).then_some(())
    });
    quest.hold(json!({"thumbstick_pressed": true}));
    until("not connected", || {
        (quest.voice()["stage"] == "not_connected").then_some(())
    });
    quest.capture("voice-not-connected.png");
    quest.hold(json!({}));
}
