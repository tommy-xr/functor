//! `functor-app device-stub`: the device role of the device link, for tests
//! and for driving the Devices pane without a headset. It pairs (printing
//! its code), links with the secret it stored, answers the debug routes
//! `functor run vr` pushes through and the app polls and drives (`GET /`,
//! `GET /state` with a clock that steps once per poll, `POST /time`, the
//! pushes) from a canned table, reporting the project a load names in its
//! heartbeats, and can be told to misbehave. One line per event on stdout:
//! `code 482913`, `paired <desktop>`, `linked <desktop>`, `request GET
//! /state` (with a body's size: `request POST /reload-asset 1024`),
//! `rejected <why>`, `ended <why>`. With `--say <wav>` it speaks that clip
//! once a game runs on it, printing each voice status it gets back: `voice 1 heard`,
//! `voice 1 transcribed <text>`, `voice 1 working`, `voice 1 reloaded`,
//! `voice 1 failed <why>`, `voice 1 idle`; with `--say-again`, the clip
//! again as utterance 2 once the first is working.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use functor_device_link::device::{self, Device, Identity};
use functor_device_link::file::Stored;
use functor_device_link::voice::{self, Capsule, Clip};
use functor_device_link::wire::{Frame, Hello, Mode, Reply, Response, Status};
use functor_device_link::{hex, os_random, shared, unhex, End, OsEntropy, Secret, SharedEntropy};
use functor_device_link::{HEARTBEAT_MS, MAX_MESSAGE, VERSION};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::link::{PATH, SERVICE};
use crate::server::now_ms;

#[derive(clap::Args, Debug)]
pub struct StubArgs {
    /// The desktop's device link, `host:port` (else found by mDNS).
    #[arg(long)]
    desktop: Option<String>,

    /// Where the stub keeps its id and pairing.
    #[arg(long, value_name = "DIR")]
    store: PathBuf,

    #[arg(long, default_value = "Device stub")]
    name: String,

    /// The headset app version it reports.
    #[arg(long, default_value = "0.1.0")]
    app_version: String,

    /// The link protocol version its hello claims.
    #[arg(long, default_value_t = VERSION)]
    protocol: u16,

    #[arg(long, value_enum)]
    misbehave: Option<Misbehave>,

    /// Exit when the first link ends instead of reconnecting.
    #[arg(long)]
    once: bool,

    /// Never ask to pair: wait until a pairing appears in the store (as
    /// one written over the cable does), then link with it.
    #[arg(long)]
    wait_for_pairing: bool,

    /// Once the desktop runs a game here (it polls `/state`), speak this
    /// clip (16 kHz mono 16-bit WAV) as utterance 1, with a capsule naming
    /// the frame.
    #[arg(long, value_name = "WAV")]
    say: Option<PathBuf>,

    /// With --say: cancel the clip halfway instead of ending it.
    #[arg(long, requires = "say")]
    cancel: bool,

    /// With --say: once the desktop says the first is working, speak the
    /// clip again as utterance 2.
    #[arg(long, requires = "say")]
    say_again: bool,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Misbehave {
    /// Link with a secret the desktop never gave.
    WrongSecret,
    /// Once linked, drop the connection without closing it.
    Drop,
    /// Once linked, go silent with the connection open.
    Stall,
    /// Once linked, send a message over the size limit.
    Oversize,
    /// Once linked, send an unreadable message on the voice channel.
    VoiceGarbage,
    /// Once linked, speak a clip longer than the voice limit.
    VoiceTooLong,
}

fn say(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub async fn run(args: StubArgs) -> std::process::ExitCode {
    // The headset app's pairing file and format.
    let file = args.store.join("device-link.json");
    let load = || Stored::parse(&std::fs::read(&file).unwrap_or_default());
    let mut stored = load();
    if unhex::<16>(&stored.device).is_none() {
        stored.device = hex(&os_random::<16>());
    }
    // Private, as a headset's pairing would be.
    let save = |stored: &Stored| {
        let _ = crate::store::make_private(&args.store);
        let _ = crate::store::write_whole(&file, &stored.to_json());
    };
    save(&stored);
    let Ok(signal) = crate::shutdown_signal() else {
        return std::process::ExitCode::FAILURE;
    };
    tokio::pin!(signal);
    // Kept across links, as a headset's runtime keeps running.
    let mut runtime = Runtime {
        project: Some("examples/hello".to_string()),
        frame: 0,
        paused: false,
        rendered: 0,
        revision: 0,
    };
    let clip = match &args.say {
        Some(wav) => match std::fs::read(wav)
            .map_err(|e| e.to_string())
            .and_then(|b| functor_stt::read_wav(&b))
        {
            Ok(pcm) => Some(pcm),
            Err(e) => {
                say(&format!("ended cannot read {}: {e}", wav.display()));
                return std::process::ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let mut said = false;
    loop {
        if args.wait_for_pairing {
            loop {
                stored = load();
                if stored.pairing().is_some() {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    _ = &mut signal => return std::process::ExitCode::SUCCESS,
                }
            }
        }
        let addr = match &args.desktop {
            Some(a) => a.clone(),
            None => match discover(stored.desktop.clone()).await {
                Some(a) => a,
                None => {
                    say("ended no desktop found");
                    return std::process::ExitCode::FAILURE;
                }
            },
        };
        let result = tokio::select! {
            r = session(&args, &mut stored, &mut runtime, &addr, clip.as_deref(), &mut said) => r,
            _ = &mut signal => return std::process::ExitCode::SUCCESS,
        };
        // Waiting for pairings written from outside, the file is theirs.
        if !args.wait_for_pairing {
            save(&stored);
        }
        match result {
            // Paired: link at once with the new secret.
            Ok(true) => continue,
            Ok(false) => say("ended"),
            Err(why) => say(&format!("ended {why}")),
        }
        if args.once {
            return std::process::ExitCode::SUCCESS;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// What the stub's debug runtime holds.
struct Runtime {
    /// The project the last `/load-project` named (`examples/hello` before one).
    project: Option<String>,
    /// `/state.frame`: one step per poll unless paused.
    frame: u64,
    paused: bool,
    /// `/state.rendered_frames`: one per poll, from 0 when the stub starts.
    rendered: u64,
    /// `/state.clock.program_revision`: one more per `/reload-project`.
    revision: u64,
}

/// One connection; `Ok(true)` when it paired.
async fn session(
    args: &StubArgs,
    stored: &mut Stored,
    runtime: &mut Runtime,
    addr: &str,
    clip: Option<&[i16]>,
    said: &mut bool,
) -> Result<bool, String> {
    let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE));
    let url = format!("ws://{addr}{PATH}");
    let (mut ws, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), true)
        .await
        .map_err(|e| e.to_string())?;
    let identity = Identity {
        device: unhex(&stored.device).expect("a valid id"),
        name: args.name.clone(),
        app_version: args.app_version.clone(),
    };
    let entropy: SharedEntropy = shared(OsEntropy);
    let now = now_ms();
    let secret = match args.misbehave {
        Some(Misbehave::WrongSecret) => Some(Secret(os_random())),
        _ => stored.secret.as_deref().and_then(Secret::from_hex),
    };
    if args.protocol != VERSION {
        return another_version(args, &identity, &mut ws).await;
    }
    let (mut machine, first) = match &secret {
        Some(secret) => Device::link(&identity, secret, status(args, runtime), entropy, now),
        None => Device::pair(&identity, entropy, now),
    };
    for msg in first.send {
        ws.send(Message::Binary(msg.into()))
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut silent = false;
    let mut served = 0u64;
    loop {
        let out = tokio::select! {
            msg = ws.next() => match msg {
                // Stalled: read and ignore, so the socket stays open.
                Some(Ok(Message::Binary(_))) if silent => continue,
                Some(Ok(Message::Binary(bytes))) => machine.receive(&bytes, now_ms()),
                Some(Ok(Message::Close(_))) | None => return Err("closed by the desktop".into()),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(e.to_string()),
            },
            _ = heartbeat.tick(), if machine.linked() && !silent => {
                let status = Frame::Status(status(args, runtime));
                let msgs = machine.send(&status).map_err(|e| e.to_string())?;
                functor_device_link::Output { send: msgs, ..Default::default() }
            }
            _ = tick.tick(), if !silent => match machine.check(now_ms()) {
                Some(e) => return Err(e.to_string()),
                None => continue,
            },
        };
        for msg in out.send {
            ws.send(Message::Binary(msg.into()))
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut paired = false;
        for event in out.events {
            match event {
                // The headset's user would compare first; the stub agrees at once.
                device::Event::Code { code, .. } => {
                    say(&format!("code {code}"));
                    for msg in machine.confirm().send {
                        ws.send(Message::Binary(msg.into()))
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
                device::Event::Paired {
                    desktop,
                    desktop_name,
                    secret,
                } => {
                    stored.secret = Some(secret.to_hex());
                    stored.desktop = Some(hex(&desktop));
                    stored.desktop_name = Some(desktop_name.clone());
                    paired = true;
                    say(&format!("paired {desktop_name}"));
                }
                device::Event::Linked { desktop_name, .. } => {
                    say(&format!("linked {desktop_name}"));
                    match args.misbehave {
                        Some(Misbehave::Drop) => {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            // Gone without a close: the socket just drops.
                            return Err("dropped".into());
                        }
                        Some(Misbehave::Stall) => silent = true,
                        Some(Misbehave::Oversize) => {
                            let big = vec![0u8; MAX_MESSAGE + 1];
                            let _ = ws.send(Message::Binary(big.into())).await;
                        }
                        _ => {}
                    }
                    let frames = match args.misbehave {
                        Some(Misbehave::VoiceGarbage) => vec![Frame::Channel {
                            channel: voice::CLIP_CHANNEL,
                            data: b"garbage".to_vec(),
                        }],
                        Some(Misbehave::VoiceTooLong) => {
                            let samples = voice::MAX_CLIP / 2 + voice::SAMPLE_RATE as usize;
                            utterance(&vec![0; samples], runtime, false, 1)
                        }
                        _ => Vec::new(),
                    };
                    for frame in frames {
                        for msg in machine.send(&frame).map_err(|e| e.to_string())? {
                            ws.send(Message::Binary(msg.into()))
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                device::Event::Request(r) => {
                    match r.body.len() {
                        0 => say(&format!("request {} {}", r.method, r.target)),
                        n => say(&format!("request {} {} {n}", r.method, r.target)),
                    }
                    served += 1;
                    let response = canned(
                        r.id, &r.method, &r.target, &r.body, &args.name, served, runtime,
                    );
                    let mut frames = vec![Frame::Response(response)];
                    // Spoken once a game runs here: the desktop polls its state.
                    if let (Some(pcm), false, "/state") =
                        (clip, *said, r.target.split('?').next().unwrap_or(""))
                    {
                        frames.extend(utterance(pcm, runtime, args.cancel, 1));
                        *said = true;
                    }
                    for frame in frames {
                        for msg in machine.send(&frame).map_err(|e| e.to_string())? {
                            ws.send(Message::Binary(msg.into()))
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                device::Event::Channel {
                    channel: voice::STATUS_CHANNEL,
                    data,
                } => match voice::Status::decode(&data) {
                    Ok(status) => {
                        say(&status_line(&status));
                        let first_working =
                            status.utterance == Some(1) && status.stage == voice::Stage::Working;
                        if let (true, true, Some(pcm)) = (args.say_again, first_working, clip) {
                            for frame in utterance(pcm, runtime, false, 2) {
                                for msg in machine.send(&frame).map_err(|e| e.to_string())? {
                                    ws.send(Message::Binary(msg.into()))
                                        .await
                                        .map_err(|e| e.to_string())?;
                                }
                            }
                        }
                    }
                    Err(e) => say(&format!("voice unreadable {e}")),
                },
                device::Event::Channel { .. } => {}
                device::Event::Rejected(reject) => say(&format!("rejected {reject:?}")),
            }
        }
        match out.end {
            None => {}
            Some(End::Done) => {
                let _ = ws.close(None).await;
                return Ok(paired);
            }
            Some(End::Failed(e)) => return Err(e.to_string()),
        }
    }
}

/// A hello claiming `args.protocol`: the desktop should refuse it by name.
async fn another_version(
    args: &StubArgs,
    identity: &Identity,
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<bool, String> {
    let mut hello = Hello {
        version: args.protocol,
        device: identity.device,
        name: identity.name.clone(),
        mode: Mode::Pair,
        app_version: identity.app_version.clone(),
    }
    .encode();
    hello.extend([0u8; 32]);
    ws.send(Message::Binary(hello.into()))
        .await
        .map_err(|e| e.to_string())?;
    match ws.next().await {
        Some(Ok(Message::Binary(bytes))) => match Reply::decode(&bytes) {
            Ok(Reply::Reject(reject)) => {
                say(&format!("rejected {reject:?}"));
                Err("another version".into())
            }
            other => Err(format!("unexpected reply {other:?}")),
        },
        other => Err(format!("no reply: {other:?}")),
    }
}

fn status(args: &StubArgs, runtime: &Runtime) -> Status {
    Status {
        battery: Some(80),
        charging: false,
        build: format!("stub-{}", args.app_version),
        project: runtime.project.clone(),
    }
}

/// The debug protocol the stub claims: `functor run vr` pushes through the
/// link only to one that takes assets in parts.
const STUB_PROTOCOL: u32 = 40;

/// The debug runtime's simplest routes and the pushes, canned: every push
/// is taken; a load sets the project its `name=` names.
fn canned(
    id: u32,
    method: &str,
    target: &str,
    body: &[u8],
    name: &str,
    served: u64,
    runtime: &mut Runtime,
) -> Response {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (status, kind, body) = match (method, path) {
        ("GET", "/") => (
            200,
            "application/json",
            serde_json::json!({
                "service": "functor debug runtime",
                "protocol_version": STUB_PROTOCOL,
            })
            .to_string(),
        ),
        ("GET", "/state") => {
            runtime.rendered += 1;
            if !runtime.paused {
                runtime.frame += 1;
            }
            let clock = serde_json::json!({
                "paused": runtime.paused,
                "recorded": { "oldest": 0, "newest": runtime.frame },
                "generation": 0,
                "program_revision": runtime.revision,
            });
            let state = serde_json::json!({
                "stub": true,
                "name": name,
                "served": served,
                "frame": runtime.frame,
                "rendered_frames": runtime.rendered,
                "clock": clock,
                "input": {},
            });
            (200, "application/json", state.to_string())
        }
        // `/time`'s commands as the app sends them: pause, resume, a step.
        ("POST", "/time") => {
            match serde_json::from_slice::<serde_json::Value>(body).ok() {
                Some(c) if c["type"] == "cancel" => runtime.paused = true,
                Some(c) if c["type"] == "resume" => runtime.paused = false,
                Some(c) if c["type"] == "advance" => runtime.frame += 1,
                _ => {}
            }
            (200, "text/plain", "ok".to_string())
        }
        ("POST", "/load-project") => {
            runtime.project = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("name="))
                .and_then(percent_decode);
            (200, "text/plain", "loaded project".to_string())
        }
        ("POST", "/reload-project") => {
            runtime.revision += 1;
            (200, "text/plain", "ok".to_string())
        }
        ("POST", "/reload-asset" | "/sync-assets") => (200, "text/plain", "ok".to_string()),
        _ => (404, "text/plain", "not found\n".to_string()),
    };
    Response {
        id,
        status,
        headers: vec![("content-type".into(), kind.into())],
        body: body.into_bytes(),
    }
}

/// `pcm` as utterance 1's frames: a start with a capsule naming the
/// frame, chunks, then its end (or, with `cancel`, a cancel halfway).
fn utterance(pcm: &[i16], runtime: &Runtime, cancel: bool, utterance: u64) -> Vec<Frame> {
    let capsule = Capsule {
        frame: Some(runtime.frame),
        pointing_at: Some("the stub".to_string()),
        ..Capsule::default()
    };
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let chunks: Vec<&[u8]> = bytes.chunks(voice::MAX_CHUNK).collect();
    let mut clips = vec![Clip::Start {
        utterance,
        capsule: capsule.to_json(),
    }];
    let sent = if cancel {
        chunks.len() / 2
    } else {
        chunks.len()
    };
    for (seq, data) in chunks[..sent].iter().enumerate() {
        clips.push(Clip::Chunk {
            utterance,
            seq: seq as u32,
            data: data.to_vec(),
        });
    }
    clips.push(match cancel {
        true => Clip::Cancel { utterance },
        false => Clip::End { utterance },
    });
    clips
        .iter()
        .map(|c| c.frame().expect("within the limits"))
        .collect()
}

/// `voice 1 transcribed make it red`.
fn status_line(status: &voice::Status) -> String {
    let u = status.utterance.map_or("-".to_string(), |u| u.to_string());
    let stage = match &status.stage {
        voice::Stage::Heard => "heard".to_string(),
        voice::Stage::Transcribed { text } => format!("transcribed {text}"),
        voice::Stage::Working => "working".to_string(),
        voice::Stage::Reloaded => "reloaded".to_string(),
        voice::Stage::Failed { reason } => format!("failed {reason}"),
        voice::Stage::Idle => "idle".to_string(),
    };
    format!("voice {u} {stage}")
}

/// `%XX` escapes decoded; `None` for a bad one or a name that is not UTF-8.
fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        out.push(match b {
            b'%' => {
                let hex = [bytes.next()?, bytes.next()?];
                u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?
            }
            b => b,
        });
    }
    String::from_utf8(out).ok()
}

/// A desktop's link address by mDNS, preferring `desktop` (its id).
async fn discover(desktop: Option<String>) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let daemon = mdns_sd::ServiceDaemon::new().ok()?;
        let found = daemon.browse(SERVICE).ok()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut first = None;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            let info = match found.recv_timeout(left) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => info,
                Ok(_) => continue,
                Err(_) => break,
            };
            let Some(ip) = info.get_addresses_v4().into_iter().next() else {
                continue;
            };
            let addr = format!("{ip}:{}", info.get_port());
            let ours = desktop.is_some() && info.get_property_val_str("id") == desktop.as_deref();
            if ours || desktop.is_none() {
                first = Some(addr);
                break;
            }
            first.get_or_insert(addr);
        }
        let _ = daemon.shutdown();
        first
    })
    .await
    .ok()
    .flatten()
}
