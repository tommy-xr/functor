//! Push-to-talk over the headset's real client and a linked stub desktop:
//! the clip arrives whole and in order, the desktop's status comes back,
//! and after a link drop nothing queued for it is resent.
#![cfg(feature = "client")]

mod stub;

use std::time::{Duration, Instant};

use functor_device_link::client::{Client, Config, Host, Stored};
use functor_device_link::talk::{Stopped, Talk, CHUNK};
use functor_device_link::voice::{Clip, Stage, Status, CLIP_CHANNEL, STATUS_CHANNEL};
use functor_device_link::wire;
use functor_device_link::{hex, Secret};

struct Quiet;

impl Host for Quiet {
    fn status(&self) -> wire::Status {
        wire::Status {
            battery: None,
            charging: false,
            build: "test".into(),
            project: None,
        }
    }
}

/// A client paired (by its file) with a stub desktop on loopback.
fn linked(dir: &std::path::Path) -> (Client, stub::Stub) {
    let (desktop, secret) = ([7u8; 16], Secret([9u8; 32]));
    let stub = stub::start("127.0.0.1:0", desktop, secret.clone());
    let store = dir.join("device-link.json");
    let mut stored = Stored::load(&store).unwrap();
    stored.desktop = Some(hex(&desktop));
    stored.desktop_name = Some("Stub".into());
    stored.secret = Some(secret.to_hex());
    stored.save(&store).unwrap();
    let config = Config {
        store,
        name: "Quest".into(),
        app_version: "0".into(),
        debug: "127.0.0.1:9".parse().unwrap(),
        desktop: Some(stub.addr.clone()),
    };
    let client = Client::spawn(config, Quiet).unwrap();
    until("linked", || client.view().linked.then_some(()));
    (client, stub)
}

fn until<T>(what: &str, f: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(t) = f() {
            return t;
        }
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_clip_crosses_and_the_status_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let (client, stub) = linked(dir.path());
    let mut talk = Talk::new(1);
    let mut sink = &client;
    let id = talk.start(16_000, b"{}".to_vec(), &mut sink).unwrap();
    // One second of a ramp, in 10 ms callbacks.
    let audio: Vec<i16> = (0..16_000).map(|i| i as i16).collect();
    for callback in audio.chunks(160) {
        assert_eq!(talk.push(callback, &mut sink), None);
    }
    assert_eq!(talk.finish(&mut sink), Some(Stopped::Sent));

    let mut got = Vec::new();
    while !matches!(got.last(), Some(Clip::End { .. })) {
        let (_, channel, data) = stub.received.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(channel, CLIP_CHANNEL);
        got.push(Clip::decode(&data).unwrap());
    }
    assert!(matches!(&got[0], Clip::Start { utterance, .. } if *utterance == id));
    let pcm: Vec<u8> = got
        .iter()
        .filter_map(|c| match c {
            Clip::Chunk { data, .. } => Some(data.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(pcm.len(), 32_000);
    assert_eq!(got.len(), 2 + 32_000usize.div_ceil(CHUNK));
    let expected: Vec<u8> = audio.iter().flat_map(|s| s.to_le_bytes()).collect();
    assert_eq!(pcm, expected, "in order, unchanged");

    let status = Status {
        utterance: Some(id),
        stage: Stage::Transcribed {
            text: "make the ball red".into(),
        },
    };
    stub.send.send(Some(status.frame())).unwrap();
    let back = until("the status", || {
        let r = client.received();
        (!r.is_empty()).then_some(r)
    });
    assert_eq!(back, vec![(STATUS_CHANNEL, status.encode())]);
    assert!(client.received().is_empty(), "taken once");
}

#[test]
fn a_dropped_link_resends_nothing_and_a_full_queue_refuses_audio() {
    let dir = tempfile::tempdir().unwrap();
    let (client, stub) = linked(dir.path());
    // Gone without a close: the client notices and relinks.
    stub.send.send(None).unwrap();
    until("unlinked", || (!client.view().linked).then_some(()));
    assert!(
        !client.send_channel(CLIP_CHANNEL, vec![1], false),
        "nothing queues while unlinked"
    );
    until("relinked", || client.view().linked.then_some(()));
    assert!(client.send_channel(CLIP_CHANNEL, vec![2], false));
    let (link, _, data) = stub.received.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!((link, data), (2, vec![2]), "only what was sent on this link");

    // Audio past the limit is refused; control messages still queue.
    let big = vec![0u8; functor_device_link::client::OUTBOX_LIMIT];
    let mut refused = false;
    for _ in 0..64 {
        refused |= !client.send_channel(CLIP_CHANNEL, big.clone(), true);
    }
    assert!(refused, "a link that cannot keep up refuses audio");
    assert!(client.send_channel(CLIP_CHANNEL, vec![3], false));
}
