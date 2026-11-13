//! A desktop stand-in for voice tests: links the one headset holding
//! `secret`, hands back each channel message it receives, sends the frames
//! it is given, and pings so the link stays up. Not the app: just enough of
//! the desktop's side of the contract.

use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use functor_device_link::desktop::{Desktop, Event};
use functor_device_link::wire::Frame;
use functor_device_link::{shared, DesktopId, OsEntropy, Secret};
use tungstenite::Message;

/// Polls `f` until it has an answer, within 30 s.
pub fn until<T>(what: &str, f: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(t) = f() {
            return t;
        }
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub struct Stub {
    /// `host:port` it listens on.
    pub addr: String,
    /// Channel messages received, with the link they arrived on (counting
    /// from 1).
    pub received: Receiver<(u32, u16, Vec<u8>)>,
    /// Frames to send on the current link; `None` drops the connection.
    pub send: Sender<Option<Frame>>,
}

/// Listens on `bind` (`127.0.0.1:0`, or `0.0.0.0:0` for a headset on the
/// network) as desktop `id`.
pub fn start(bind: &str, id: DesktopId, secret: Secret) -> Stub {
    let listener = TcpListener::bind(bind).unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (received_tx, received) = mpsc::channel();
    let (send, frames) = mpsc::channel();
    std::thread::spawn(move || {
        for (link, stream) in (1..).zip(listener.incoming()) {
            let Ok(stream) = stream else { continue };
            serve(stream, link, id, &secret, &received_tx, &frames);
        }
    });
    Stub {
        addr,
        received,
        send,
    }
}

fn serve(
    stream: TcpStream,
    link: u32,
    id: DesktopId,
    secret: &Secret,
    received: &Sender<(u32, u16, Vec<u8>)>,
    frames: &Receiver<Option<Frame>>,
) {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let Ok(mut ws) = tungstenite::accept(stream) else {
        return;
    };
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let mut machine = Desktop::new(id, "Stub".into(), shared(OsEntropy), now());
    let mut next_ping = 0;
    loop {
        match ws.read() {
            Ok(Message::Binary(bytes)) => {
                let out = machine.receive(&bytes, now(), |_| Some(secret.clone()));
                for msg in out.send {
                    let _ = ws.send(Message::Binary(msg.into()));
                }
                for event in out.events {
                    if let Event::Channel { channel, data } = event {
                        let _ = received.send((link, channel, data));
                    }
                }
                if out.end.is_some() {
                    return;
                }
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
        if !machine.linked() {
            continue;
        }
        while let Ok(frame) = frames.try_recv() {
            // Dropped without a close, as a lost network would.
            let Some(frame) = frame else { return };
            for msg in machine.send(&frame).unwrap() {
                let _ = ws.send(Message::Binary(msg.into()));
            }
        }
        if now() >= next_ping {
            next_ping = now() + 2_000;
            for msg in machine.send(&Frame::Ping(now())).unwrap() {
                let _ = ws.send(Message::Binary(msg.into()));
            }
        }
    }
}
