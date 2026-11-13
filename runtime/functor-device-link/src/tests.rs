use crate::desktop::{self, Desktop};
use crate::device::{self, Device, Identity};
use crate::wire::{Frame, Hello, Mode, Reject, Reply, Request, Response, Status};
use crate::*;

/// Deterministic randomness (splitmix64), so every run is the same.
struct Seeded(u64);

impl Entropy for Seeded {
    fn fill(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_be_bytes()[..chunk.len()]);
        }
    }
}

fn seeded(seed: u64) -> SharedEntropy {
    shared(Seeded(seed))
}

const DESKTOP: DesktopId = [7; 16];

fn identity() -> Identity {
    Identity {
        device: [1; 16],
        name: "Quest 3".to_string(),
        app_version: "0.1.0".to_string(),
    }
}

fn status() -> Status {
    Status {
        battery: Some(80),
        charging: false,
        build: "abc123".to_string(),
        project: Some("examples/hello".to_string()),
    }
}

fn new_desktop(seed: u64) -> Desktop {
    Desktop::new(DESKTOP, "studio".to_string(), seeded(seed), 0)
}

/// Delivers `msgs` to the desktop, gathering its output.
fn to_desktop(
    desktop: &mut Desktop,
    msgs: &[Vec<u8>],
    now: u64,
    secret: Option<&Secret>,
) -> Output<desktop::Event> {
    let mut all = Output::default();
    for msg in msgs {
        let out = desktop.receive(msg, now, |_| secret.cloned());
        all.send.extend(out.send);
        all.events.extend(out.events);
        if out.end.is_some() {
            all.end = out.end;
            break;
        }
    }
    all
}

fn to_device(device: &mut Device, msgs: &[Vec<u8>], now: u64) -> Output<device::Event> {
    let mut all = Output::default();
    for msg in msgs {
        let out = device.receive(msg, now);
        all.send.extend(out.send);
        all.events.extend(out.events);
        if out.end.is_some() {
            all.end = out.end;
            break;
        }
    }
    all
}

/// A pairing up to both showing their codes: (desktop, device, desktop's
/// code, device's code, the device's first message).
fn compare(seed: u64) -> (Desktop, Device, String, String, Vec<u8>) {
    let mut desktop = new_desktop(seed);
    let (mut device, hello) = Device::pair(&identity(), seeded(seed + 1), 0);
    let first = hello.send[0].clone();
    let reply = to_desktop(&mut desktop, &hello.send, 1, None);
    assert_eq!(reply.end, None);
    let nonce = to_device(&mut device, &reply.send, 2);
    let reveal = to_desktop(&mut desktop, &nonce.send, 3, None);
    let [desktop::Event::PairRequest { code: a, hello, .. }] = &reveal.events[..] else {
        panic!("no pair request: {:?}", reveal.events);
    };
    assert_eq!(hello.name, "Quest 3");
    let shown = to_device(&mut device, &reveal.send, 4);
    let [device::Event::Code {
        code: b,
        desktop_name,
        ..
    }] = &shown.events[..]
    else {
        panic!("no code: {:?}", shown.events);
    };
    assert_eq!(desktop_name, "studio");
    (desktop, device, a.clone(), b.clone(), first)
}

/// Pairs, returning the secret both stored.
fn paired(seed: u64) -> Secret {
    let (mut desktop, mut device, _, _, _) = compare(seed);
    // The headset's user confirms first; the desktop's Allow sends the secret.
    let confirm = device.confirm();
    assert!(to_desktop(&mut desktop, &confirm.send, 5, None)
        .events
        .is_empty());
    let allowed = desktop.allow(5);
    let [desktop::Event::Paired { secret, .. }] = &allowed.events[..] else {
        panic!("not paired: {:?}", allowed.events);
    };
    let stored = to_device(&mut device, &allowed.send, 6);
    let [device::Event::Paired {
        secret: theirs,
        desktop: id,
        ..
    }] = &stored.events[..]
    else {
        panic!("device not paired: {:?}", stored.events);
    };
    assert_eq!(*id, DESKTOP);
    assert_eq!(theirs, secret);
    assert_eq!(stored.end, Some(End::Done));
    let done = to_desktop(&mut desktop, &stored.send, 7, None);
    assert_eq!(done.end, Some(End::Done));
    secret.clone()
}

/// A linked pair at time 10.
fn linked(seed: u64, secret: &Secret) -> (Desktop, Device) {
    let mut desktop = new_desktop(seed);
    let (mut device, hello) = Device::link(&identity(), secret, status(), seeded(seed + 1), 0);
    let reply = to_desktop(&mut desktop, &hello.send, 1, Some(secret));
    assert!(
        reply.events.is_empty(),
        "not linked before the device speaks"
    );
    assert!(!desktop.linked());
    let up = to_device(&mut device, &reply.send, 2);
    assert!(
        matches!(&up.events[..], [device::Event::Linked { desktop, .. }] if *desktop == DESKTOP)
    );
    let first = to_desktop(&mut desktop, &up.send, 3, Some(secret));
    assert!(
        matches!(&first.events[..], [desktop::Event::Linked { .. }, desktop::Event::Status(s)] if *s == status()),
        "{:?}",
        first.events
    );
    assert!(desktop.linked() && device.linked());
    (desktop, device)
}

// ---- framing ----

#[test]
fn every_frame_round_trips() {
    let frames = [
        Frame::Request(Request {
            id: 9,
            method: "POST".into(),
            target: "/frames?count=3".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{\"a\":1}".to_vec(),
        }),
        Frame::Response(Response {
            id: 9,
            status: 404,
            headers: vec![],
            body: vec![0, 1, 2, 255],
        }),
        Frame::Status(status()),
        Frame::Status(Status::default()),
        Frame::Ping(u64::MAX),
        Frame::Pong(3),
        Frame::Channel {
            channel: 2,
            data: vec![5; 10],
        },
        Frame::Close("bye".into()),
        Frame::PairNonce([3; 32]),
        Frame::PairReveal([4; 32]),
        Frame::PairAccept {
            desktop: [5; 16],
            secret: Secret([6; 32]),
        },
        Frame::PairDone,
        Frame::PairDeny,
        Frame::PairCancel,
    ];
    for frame in frames {
        assert_eq!(
            Frame::decode(&frame.encode()),
            Ok(frame.clone()),
            "{frame:?}"
        );
    }
}

#[test]
fn hello_and_reply_round_trip() {
    let hello = Hello {
        version: VERSION,
        device: [9; 16],
        name: "Quest 3".into(),
        mode: Mode::Link,
        app_version: "1.2.3".into(),
    };
    let mut bytes = hello.encode();
    let len = bytes.len();
    bytes.extend([1, 2, 3]);
    assert_eq!(Hello::decode(&bytes), Ok((hello, len)));
    for reply in [
        Reply::Handshake(vec![1, 2, 3]),
        Reply::Reject(Reject::Version(7)),
        Reply::Reject(Reject::NotPaired),
        Reply::Reject(Reject::Busy),
    ] {
        assert_eq!(Reply::decode(&reply.encode()), Ok(reply.clone()));
    }
}

#[test]
fn decoding_enforces_limits() {
    // A name over MAX_NAME: magic, version and id are 22 bytes, then its length.
    let mut long = Hello {
        version: VERSION,
        device: [0; 16],
        name: String::new(),
        mode: Mode::Pair,
        app_version: "1".into(),
    }
    .encode();
    long[22] = 65;
    long.splice(23..23, vec![b'a'; 65]);
    assert!(Hello::decode(&long).is_err());
    // Control characters in text.
    let name = Frame::Close("a\u{1b}[2J".into()).encode();
    assert_eq!(Frame::decode(&name), Err(Error::Malformed("text")));
    // A body length beyond MAX_FRAME, a battery over 100, trailing bytes.
    let mut body = vec![6, 0, 1];
    body.extend(((MAX_FRAME + 1) as u32).to_be_bytes());
    assert_eq!(Frame::decode(&body), Err(Error::TooLarge));
    assert_eq!(
        Frame::decode(&[3, 101, 0, 0, 0, 0]),
        Err(Error::Malformed("battery"))
    );
    assert_eq!(
        Frame::decode(&[19, 0]),
        Err(Error::Malformed("trailing bytes"))
    );
    assert_eq!(Frame::decode(&[99]), Err(Error::Malformed("frame kind")));
    assert_eq!(Frame::decode(&[]), Err(Error::Malformed("truncated")));
}

#[test]
fn a_large_body_crosses_in_chunks_and_an_oversize_one_is_refused() {
    let secret = paired(10);
    let (mut desktop, mut device) = linked(20, &secret);
    let body: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
    let msgs = desktop
        .request(Request {
            id: 1,
            method: "POST".into(),
            target: "/reload-source".into(),
            headers: vec![],
            body: body.clone(),
        })
        .unwrap();
    assert!(msgs.len() > 4 && msgs.iter().all(|m| m.len() <= MAX_MESSAGE));
    let got = to_device(&mut device, &msgs, 11);
    assert!(matches!(&got.events[..], [device::Event::Request(r)] if r.body == body));
    let big = device.respond(Response {
        id: 1,
        status: 200,
        headers: vec![],
        body: vec![0; MAX_FRAME + 1],
    });
    assert_eq!(big, Err(Error::TooLarge));
    // A message over MAX_MESSAGE ends the link.
    let out = desktop.receive(&vec![0; MAX_MESSAGE + 1], 12, |_| None);
    assert_eq!(out.end, Some(End::Failed(Error::TooLarge)));
}

#[test]
fn another_version_is_refused_by_name_with_ours() {
    let mut desktop = new_desktop(1);
    let mut hello = Hello {
        version: VERSION + 1,
        device: [4; 16],
        name: "Quest Pro".into(),
        mode: Mode::Pair,
        app_version: "9".into(),
    }
    .encode();
    // A later version may change everything after the preamble.
    hello.truncate(4 + 2 + 16 + 1 + "Quest Pro".len());
    hello.extend([0xee; 40]);
    let out = desktop.receive(&hello, 0, |_| None);
    let [desktop::Event::Mismatch(pre)] = &out.events[..] else {
        panic!("{:?}", out.events);
    };
    assert_eq!(
        (pre.version, pre.device, pre.name.as_str()),
        (VERSION + 1, [4; 16], "Quest Pro")
    );
    assert_eq!(
        out.end,
        Some(End::Failed(Error::Version {
            theirs: VERSION + 1,
            ours: VERSION
        }))
    );
    // The device side of the same refusal.
    let (mut device, _) = Device::pair(&identity(), seeded(2), 0);
    let got = to_device(&mut device, &out.send, 1);
    assert_eq!(
        got.events,
        [device::Event::Rejected(Reject::Version(VERSION))]
    );
    assert!(matches!(
        got.end,
        Some(End::Failed(Error::Version {
            theirs: VERSION,
            ..
        }))
    ));
}

// ---- pairing ----

#[test]
fn both_sides_derive_the_same_code_and_secret() {
    let (_, _, a, b, _) = compare(100);
    assert_eq!(a, b);
    assert_eq!(a.len(), CODE_DIGITS);
    assert!(a.bytes().all(|c| c.is_ascii_digit()));
    paired(100);
}

#[test]
fn codes_differ_between_runs() {
    let codes: std::collections::HashSet<String> =
        (0..20).map(|i| compare(1000 + i * 10).2).collect();
    assert!(codes.len() >= 19, "{codes:?}");
}

#[test]
fn a_machine_in_the_middle_sees_different_codes_on_each_side() {
    // device <-> mitm_desktop | mitm_device <-> desktop: two handshakes.
    let mut desktop = new_desktop(1);
    let mut mitm_desktop = Desktop::new(DESKTOP, "studio".into(), seeded(66), 0);
    let (mut device, hello) = Device::pair(&identity(), seeded(2), 0);
    let (mut mitm_device, mitm_hello) = Device::pair(&identity(), seeded(67), 0);
    let r1 = to_desktop(&mut mitm_desktop, &hello.send, 1, None);
    let n1 = to_device(&mut device, &r1.send, 1);
    let r2 = to_desktop(&mut desktop, &mitm_hello.send, 1, None);
    let n2 = to_device(&mut mitm_device, &r2.send, 1);
    let v1 = to_desktop(&mut mitm_desktop, &n1.send, 2, None);
    let v2 = to_desktop(&mut desktop, &n2.send, 2, None);
    let c1 = to_device(&mut device, &v1.send, 3);
    let c2 = to_device(&mut mitm_device, &v2.send, 3);
    let device_code = |events: &[device::Event]| match events {
        [device::Event::Code { code, .. }] => code.clone(),
        other => panic!("{other:?}"),
    };
    let desktop_code = |events: &[desktop::Event]| match events {
        [desktop::Event::PairRequest { code, .. }] => code.clone(),
        other => panic!("{other:?}"),
    };
    // Each leg agrees with itself; the two legs, which the user compares,
    // do not.
    assert_eq!(device_code(&c1.events), desktop_code(&v1.events));
    assert_eq!(device_code(&c2.events), desktop_code(&v2.events));
    assert_ne!(device_code(&c1.events), desktop_code(&v2.events));

    // Forwarding a desktop's answer to another device's hello fails: the
    // keys differ.
    let (mut device, _) = Device::pair(&identity(), seeded(3), 0);
    let mut desktop = new_desktop(4);
    let (_, other_hello) = Device::pair(&identity(), seeded(5), 0);
    let reply = to_desktop(&mut desktop, &other_hello.send, 1, None);
    let out = to_device(&mut device, &reply.send, 2);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));
}

#[test]
fn a_desktop_revealing_another_nonce_than_it_committed_to_is_refused() {
    use crate::crypto;
    let (mut device, hello) = Device::pair(&identity(), seeded(150), 0);
    let first = &hello.send[0];
    let (_, at) = Hello::decode(first).unwrap();
    // A rogue desktop, built from the primitives: commits to one nonce...
    let entropy = seeded(151);
    let mut hs = crypto::handshake(Mode::Pair, false, &first[..at], None, &entropy);
    crypto::read(&mut hs, &first[at..]).unwrap();
    let payload = crate::wire::encode_desktop(&DESKTOP, "rogue", Some(&crypto::commit(&[1; 32])));
    let reply = Reply::Handshake(crypto::write(&mut hs, &payload)).encode();
    let mut transport = crypto::Transport::new(hs).unwrap();
    to_device(&mut device, &[reply], 1);
    // ...and, having seen the device's, reveals another.
    let reveal = transport.seal(&Frame::PairReveal([2; 32])).unwrap();
    let out = to_device(&mut device, &reveal, 2);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));
    assert!(out.events.is_empty());
}

#[test]
fn the_secret_waits_for_both_users_in_either_order() {
    // Allowed on the desktop first: nothing is sent until the headset confirms.
    let (mut desktop, mut device, _, _, _) = compare(160);
    let early = desktop.allow(5);
    assert!(early.send.is_empty() && early.events.is_empty() && early.end.is_none());
    let confirmed = to_desktop(&mut desktop, &device.confirm().send, 6, None);
    assert!(matches!(
        &confirmed.events[..],
        [desktop::Event::Paired { .. }]
    ));
    let stored = to_device(&mut device, &confirmed.send, 7);
    assert!(matches!(&stored.events[..], [device::Event::Paired { .. }]));
}

#[test]
fn a_desktop_that_accepts_before_the_headset_confirms_is_refused() {
    use crate::crypto;
    // A rogue desktop answers its own prompt the moment the code shows.
    let (mut device, hello) = Device::pair(&identity(), seeded(170), 0);
    let first = &hello.send[0];
    let (_, at) = Hello::decode(first).unwrap();
    let entropy = seeded(171);
    let mut hs = crypto::handshake(Mode::Pair, false, &first[..at], None, &entropy);
    crypto::read(&mut hs, &first[at..]).unwrap();
    let nonce = [9; 32];
    let payload = crate::wire::encode_desktop(&DESKTOP, "rogue", Some(&crypto::commit(&nonce)));
    let reply = Reply::Handshake(crypto::write(&mut hs, &payload)).encode();
    let mut transport = crypto::Transport::new(hs).unwrap();
    to_device(&mut device, &[reply], 1);
    let shown = to_device(
        &mut device,
        &transport.seal(&Frame::PairReveal(nonce)).unwrap(),
        2,
    );
    assert!(matches!(&shown.events[..], [device::Event::Code { .. }]));
    let accept = Frame::PairAccept {
        desktop: DESKTOP,
        secret: Secret([5; 32]),
    };
    let out = to_device(&mut device, &transport.seal(&accept).unwrap(), 3);
    assert!(out.events.is_empty(), "{:?}", out.events);
    assert!(matches!(out.end, Some(End::Failed(Error::Unexpected(_)))));
}

#[test]
fn strangers_get_no_big_buffers_before_the_link() {
    use crate::crypto;
    // A rogue device, built from the primitives, pairs far enough to have
    // a transport, then streams a big frame.
    let hello = Hello {
        version: VERSION,
        device: [3; 16],
        name: "rogue".into(),
        mode: Mode::Pair,
        app_version: "1".into(),
    }
    .encode();
    let entropy = seeded(180);
    let mut hs = crypto::handshake(Mode::Pair, true, &hello, None, &entropy);
    let mut first = hello;
    first.extend(crypto::write(&mut hs, &[]));
    let mut desktop = new_desktop(181);
    let reply = to_desktop(&mut desktop, &[first], 1, None);
    let Ok(Reply::Handshake(msg2)) = Reply::decode(&reply.send[0]) else {
        panic!("no handshake reply");
    };
    crypto::read(&mut hs, &msg2).unwrap();
    let mut transport = crypto::Transport::new(hs).unwrap();
    let big = Frame::Channel {
        channel: 1,
        data: vec![0; MAX_PAIRING_FRAME],
    };
    let out = to_desktop(&mut desktop, &transport.seal(&big).unwrap(), 2, None);
    assert_eq!(out.end, Some(End::Failed(Error::TooLarge)));
}

#[test]
fn a_frame_the_peer_would_refuse_is_not_sent() {
    let secret = paired(190);
    let (mut desktop, _) = linked(191, &secret);
    let long = Request {
        id: 1,
        method: "GET".into(),
        target: format!("/{}", "a".repeat(crate::wire::MAX_TARGET)),
        headers: vec![],
        body: vec![],
    };
    assert_eq!(desktop.request(long), Err(Error::TooLarge));
    let hidden = Frame::Close("a\u{202e}b".into());
    assert_eq!(desktop.send(&hidden), Err(Error::Malformed("text")));
    // And the link is still up.
    assert!(desktop.linked());
}

#[test]
fn a_denied_pairing_stores_nothing() {
    let (mut desktop, mut device, _, _, _) = compare(200);
    let out = desktop.deny();
    assert!(out.events.is_empty());
    assert_eq!(out.end, Some(End::Failed(Error::Denied)));
    let got = to_device(&mut device, &out.send, 6);
    assert!(got.events.is_empty());
    assert_eq!(got.end, Some(End::Failed(Error::Denied)));
}

#[test]
fn an_expired_code_cannot_be_allowed() {
    let (mut desktop, mut device, _, _, _) = compare(300);
    // The code was shown at 3 (desktop) and 4 (device).
    let late = 3 + PAIR_EXPIRY_MS;
    to_desktop(&mut desktop, &device.confirm().send, 5, None);
    let out = desktop.allow(late);
    assert!(out.events.is_empty() && out.send.is_empty());
    assert_eq!(out.end, Some(End::Failed(Error::Expired)));
    assert_eq!(device.check(late), None);
    assert_eq!(device.check(4 + PAIR_EXPIRY_MS), Some(Error::Expired));
    let (mut desktop, _, _, _, _) = compare(301);
    assert_eq!(desktop.check(late - 1), None);
    assert_eq!(desktop.check(late), Some(Error::Expired));
}

#[test]
fn the_device_can_cancel() {
    let (mut desktop, mut device, _, _, _) = compare(350);
    let out = device.cancel();
    assert_eq!(out.end, Some(End::Failed(Error::Cancelled)));
    let got = to_desktop(&mut desktop, &out.send, 5, None);
    assert_eq!(got.end, Some(End::Failed(Error::Cancelled)));
    // Allowing afterwards does nothing.
    let late = desktop.allow(6);
    assert!(late.events.is_empty() && late.send.is_empty() && late.end.is_none());
}

#[test]
fn a_replayed_pairing_handshake_goes_nowhere() {
    // A recorded run: the device's hello and its nonce message.
    let mut recorded = new_desktop(400);
    let (mut device, hello) = Device::pair(&identity(), seeded(401), 0);
    let reply = to_desktop(&mut recorded, &hello.send, 1, None);
    let nonce = to_device(&mut device, &reply.send, 2);
    // Replayed to a fresh desktop: it answers the hello, but the replayer
    // holds no ephemeral key, so the nonce message cannot decrypt.
    let mut desktop = new_desktop(402);
    assert_eq!(to_desktop(&mut desktop, &hello.send, 0, None).end, None);
    let out = to_desktop(&mut desktop, &nonce.send, 1, None);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));
    assert!(out.events.is_empty());
}

#[test]
fn unanswered_handshakes_time_out() {
    let mut desktop = new_desktop(1);
    assert_eq!(desktop.check(HANDSHAKE_TIMEOUT_MS - 1), None);
    assert_eq!(desktop.check(HANDSHAKE_TIMEOUT_MS), Some(Error::Timeout));
    let (mut device, _) = Device::pair(&identity(), seeded(2), 0);
    assert_eq!(device.check(HANDSHAKE_TIMEOUT_MS), Some(Error::Timeout));
    assert_eq!(device.receive(&[0], 1).end, Some(End::Failed(Error::Ended)));
}

// ---- linking ----

#[test]
fn a_paired_device_links_and_carries_requests() {
    let secret = paired(500);
    let (mut desktop, mut device) = linked(510, &secret);
    let req = Request {
        id: 4,
        method: "GET".into(),
        target: "/state".into(),
        headers: vec![],
        body: vec![],
    };
    let got = to_device(&mut device, &desktop.request(req.clone()).unwrap(), 11);
    assert_eq!(got.events, [device::Event::Request(req)]);
    let res = Response {
        id: 4,
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body: b"{}".to_vec(),
    };
    let back = to_desktop(
        &mut desktop,
        &device.respond(res.clone()).unwrap(),
        12,
        None,
    );
    assert_eq!(back.events, [desktop::Event::Response(res)]);
    // A ping is answered by the machine itself.
    let pong = to_device(&mut device, &desktop.send(&Frame::Ping(100)).unwrap(), 120);
    let latency = to_desktop(&mut desktop, &pong.send, 142, None);
    assert_eq!(latency.events, [desktop::Event::Latency(42)]);
    // Heartbeats keep it up; silence ends it.
    let beat = device.send(&Frame::Status(status())).unwrap();
    to_desktop(&mut desktop, &beat, 1000, None);
    assert_eq!(desktop.check(1000 + LINK_TIMEOUT_MS - 1), None);
    assert_eq!(desktop.check(1000 + LINK_TIMEOUT_MS), Some(Error::Timeout));
    assert_eq!(device.check(120 + LINK_TIMEOUT_MS), Some(Error::Timeout));
}

#[test]
fn a_clean_close_is_done() {
    let secret = paired(520);
    let (mut desktop, mut device) = linked(530, &secret);
    let bye = device.send(&Frame::Close("sleeping".into())).unwrap();
    assert_eq!(
        to_desktop(&mut desktop, &bye, 11, None).end,
        Some(End::Done)
    );
    // Except a pairing replaced: the desktop's user is told why. The
    // reason is matched exactly, so it is pinned.
    assert_eq!(crate::REPAIRED, "the headset's pairing changed");
    let (mut desktop, mut device) = linked(540, &secret);
    let bye = device.send(&Frame::Close(crate::REPAIRED.into())).unwrap();
    assert_eq!(
        to_desktop(&mut desktop, &bye, 11, None).end,
        Some(End::Failed(Error::Closed(crate::REPAIRED.into())))
    );
}

#[test]
fn the_wrong_secret_or_an_unknown_device_is_refused() {
    let secret = paired(600);
    for presented in [Secret([0; 32]), Secret([1; 32])] {
        let mut desktop = new_desktop(601);
        let (mut device, hello) = Device::link(&identity(), &presented, status(), seeded(602), 0);
        let out = to_desktop(&mut desktop, &hello.send, 1, Some(&secret));
        assert_eq!(out.end, Some(End::Failed(Error::NotPaired)));
        assert!(out.events.is_empty());
        let got = to_device(&mut device, &out.send, 2);
        assert_eq!(got.events, [device::Event::Rejected(Reject::NotPaired)]);
    }
    let mut desktop = new_desktop(603);
    let (_, hello) = Device::link(&identity(), &secret, status(), seeded(604), 0);
    let out = to_desktop(&mut desktop, &hello.send, 1, None);
    assert_eq!(out.end, Some(End::Failed(Error::NotPaired)));
}

#[test]
fn a_tampered_dropped_or_replayed_frame_ends_the_link() {
    let secret = paired(700);
    let (mut desktop, mut device) = linked(710, &secret);
    let beat = device.send(&Frame::Status(status())).unwrap();
    let mut tampered = beat[0].clone();
    tampered[3] ^= 1;
    let out = desktop.receive(&tampered, 11, |_| None);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));

    let (mut desktop, mut device) = linked(720, &secret);
    let beat = device.send(&Frame::Status(status())).unwrap();
    assert_eq!(desktop.receive(&beat[0], 11, |_| None).end, None);
    let out = desktop.receive(&beat[0], 12, |_| None);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));

    let (mut desktop, mut device) = linked(730, &secret);
    let _lost = device.send(&Frame::Status(status())).unwrap();
    let next = device.send(&Frame::Status(status())).unwrap();
    let out = desktop.receive(&next[0], 11, |_| None);
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));
}

#[test]
fn a_replayed_link_handshake_never_links() {
    let secret = paired(800);
    let (mut device, hello) = Device::link(&identity(), &secret, status(), seeded(801), 0);
    let mut first = new_desktop(802);
    let reply = to_desktop(&mut first, &hello.send, 1, Some(&secret));
    let status_msgs = to_device(&mut device, &reply.send, 2).send;
    // Both of the device's messages, replayed to a new connection.
    let mut replayed = new_desktop(803);
    let out = to_desktop(&mut replayed, &hello.send, 10, Some(&secret));
    assert_eq!(out.end, None);
    let out = to_desktop(&mut replayed, &status_msgs, 11, Some(&secret));
    assert_eq!(out.end, Some(End::Failed(Error::Crypto)));
    assert!(out.events.is_empty());
    assert!(!replayed.linked());
    // The hello alone: it times out unlinked.
    let mut idle = new_desktop(804);
    to_desktop(&mut idle, &hello.send, 10, Some(&secret));
    assert_eq!(idle.check(10 + HANDSHAKE_TIMEOUT_MS), Some(Error::Timeout));
}

#[test]
fn a_device_cannot_make_requests_or_send_desktop_frames() {
    let secret = paired(900);
    let request = Frame::Request(Request {
        id: 1,
        method: "GET".into(),
        target: "/".into(),
        headers: vec![],
        body: vec![],
    });
    let (mut desktop, mut device) = linked(910, &secret);
    // Neither machine sends a frame of the other's direction.
    assert!(device.send(&request).is_err());
    assert!(desktop.send(&Frame::Status(status())).is_err());
    assert!(desktop.send(&Frame::PairDone).is_err());
    // Sent anyway, the desktop closes the link.
    for forged in [
        request,
        Frame::Ping(1),
        Frame::PairAccept {
            desktop: [0; 16],
            secret: Secret([0; 32]),
        },
    ] {
        let (mut desktop, mut device) = linked(920, &secret);
        let out = to_desktop(&mut desktop, &device.send_raw(&forged), 11, None);
        assert!(
            matches!(out.end, Some(End::Failed(Error::Unexpected(_)))),
            "{forged:?}: {out:?}"
        );
        assert!(out.events.is_empty());
    }
}

// ---- garbage ----

#[test]
fn garbage_and_truncated_input_never_panics() {
    let mut rng = Seeded(42);
    let mut garbage = |n: usize| {
        let mut out = vec![0; n];
        rng.fill(&mut out);
        out
    };
    let lengths = [
        0, 1, 2, 3, 4, 5, 6, 20, 22, 23, 40, 48, 64, 100, 500, 1024, 1025, 70_000,
    ];
    let key = Secret([1; 32]);
    // Fresh machines, random bytes.
    for &n in &lengths {
        for _ in 0..8 {
            let bytes = garbage(n);
            let mut desktop = new_desktop(1);
            let _ = desktop.receive(&bytes, 0, |_| Some(key.clone()));
            let _ = desktop.receive(&bytes, 1, |_| None);
            let (mut device, _) = Device::pair(&identity(), seeded(2), 0);
            let _ = device.receive(&bytes, 0);
            let (mut device, _) = Device::link(&identity(), &key, status(), seeded(3), 0);
            let _ = device.receive(&bytes, 0);
        }
    }
    // Every truncation and single-bit flip of real handshake messages.
    let (mut device, hello) = Device::pair(&identity(), seeded(5), 0);
    let first = hello.send[0].clone();
    let reply = new_desktop(6).receive(&first, 0, |_| None).send[0].clone();
    let nonce = device.receive(&reply, 1).send[0].clone();
    for msg in [&first, &reply, &nonce] {
        for cut in 0..msg.len() {
            let _ = new_desktop(7).receive(&msg[..cut], 0, |_| None);
            let (mut d, _) = Device::pair(&identity(), seeded(8), 0);
            let _ = d.receive(&msg[..cut], 0);
        }
        for bit in 0..msg.len() * 8 {
            let mut flipped = msg.clone();
            flipped[bit / 8] ^= 1 << (bit % 8);
            let out = new_desktop(7).receive(&flipped, 0, |_| None);
            // A first message alone never pairs or links.
            assert!(out
                .events
                .iter()
                .all(|e| matches!(e, desktop::Event::Mismatch(_))));
            let (mut d, _) = Device::pair(&identity(), seeded(8), 0);
            let _ = d.receive(&flipped, 0);
        }
    }
    // Garbage into linked machines ends them.
    let secret = paired(9);
    for &n in &lengths {
        let (mut desktop, mut device) = linked(10, &secret);
        let bytes = garbage(n);
        assert!(desktop.receive(&bytes, 11, |_| None).end.is_some());
        assert!(device.receive(&bytes, 11).end.is_some());
    }
    // Garbage straight into the decoders.
    let mut rng = Seeded(43);
    for _ in 0..5000 {
        let mut len = [0u8; 1];
        rng.fill(&mut len);
        let mut bytes = vec![0; len[0] as usize % 80];
        rng.fill(&mut bytes);
        let _ = Frame::decode(&bytes);
        let _ = Reply::decode(&bytes);
        let _ = Hello::decode(&bytes);
    }
}

#[test]
fn hex_round_trips() {
    let secret = Secret([0xab; 32]);
    assert_eq!(Secret::from_hex(&secret.to_hex()), Some(secret.clone()));
    assert_eq!(Secret::from_hex("zz"), None);
    assert_eq!(unhex::<2>("0aF0"), Some([0x0a, 0xf0]));
    assert_eq!(unhex::<2>("0a"), None);
    assert_eq!(format!("{secret:?}"), "Secret(..)");
}
