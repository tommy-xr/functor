//! The desktop's side of one connection: answers the hello, pairs (showing
//! the code until its user allows or denies) or authenticates a paired
//! device, then carries the link.

use crate::crypto::{self, Transport};
use crate::wire::{self, Frame, Hello, HelloError, Mode, Preamble, Reject, Reply, Request};
use crate::wire::{Response, Status};
use crate::{random, DesktopId, DeviceId, End, Error, Output, Secret, SharedEntropy};
use crate::{HANDSHAKE_TIMEOUT_MS, LINK_TIMEOUT_MS, MAX_FRAME, PAIR_EXPIRY_MS, VERSION};

/// A hello plus its handshake message fits well within this.
const MAX_FIRST: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// The device asks to pair: show `code` (until `expires_at`) and ask the
    /// user to [`Desktop::allow`] or [`Desktop::deny`].
    PairRequest {
        hello: Hello,
        code: String,
        expires_at: u64,
    },
    /// Allowed: store `secret` for `hello.device` now. The connection ends
    /// once the device has stored it too.
    Paired {
        hello: Hello,
        secret: Secret,
    },
    /// A paired device proved its secret: the link is up.
    Linked {
        hello: Hello,
    },
    Response(Response),
    Status(Status),
    /// Round trip of the last ping, in the clock's milliseconds.
    Latency(u64),
    Channel {
        channel: u16,
        data: Vec<u8>,
    },
    /// The device speaks another version; only its preamble is known.
    Mismatch(Preamble),
}

enum Phase {
    Hello,
    /// Pairing: our handshake message (with the commitment) is sent; the
    /// device's nonce is next.
    Committed {
        transport: Transport,
        hello: Hello,
        nonce: [u8; 32],
        hash: Vec<u8>,
    },
    /// Both sides show the code. The secret goes once both users said it
    /// matches: ours (`allowed`) and the headset's (`confirmed`).
    Comparing {
        transport: Transport,
        hello: Hello,
        allowed: bool,
        confirmed: bool,
    },
    /// Allowed; the device stores the secret and says so.
    Accepted {
        transport: Transport,
    },
    /// Linking: our handshake message is sent; the device's first transport
    /// message proves it holds the keys (a replayed hello cannot).
    Linking {
        transport: Transport,
        hello: Hello,
    },
    Linked {
        transport: Transport,
    },
    Ended,
}

pub struct Desktop {
    id: DesktopId,
    name: String,
    entropy: SharedEntropy,
    phase: Phase,
    /// When the current phase times out (pairing: the code expires).
    deadline: u64,
}

impl Desktop {
    /// A fresh connection accepted at `now`, for the desktop `id` named `name`.
    pub fn new(id: DesktopId, name: String, entropy: SharedEntropy, now: u64) -> Desktop {
        Desktop {
            id,
            name,
            entropy,
            phase: Phase::Hello,
            deadline: now + HANDSHAKE_TIMEOUT_MS,
        }
    }

    /// Whether the link is up (authenticated).
    pub fn linked(&self) -> bool {
        matches!(self.phase, Phase::Linked { .. })
    }

    /// One received WebSocket message. `secret` finds a paired device's
    /// pairing secret.
    pub fn receive(
        &mut self,
        msg: &[u8],
        now: u64,
        secret: impl FnOnce(&DeviceId) -> Option<Secret>,
    ) -> Output<Event> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Hello => self.hello_message(msg, now, secret),
            Phase::Ended => Output::failed(Error::Ended),
            phase => match self.open(phase, msg) {
                Ok((phase, None)) => {
                    self.phase = phase;
                    Output::default()
                }
                Ok((phase, Some(frame))) => self.frame(phase, frame, now),
                Err(e) => Output::failed(e),
            },
        }
    }

    fn hello_message(
        &mut self,
        msg: &[u8],
        now: u64,
        secret: impl FnOnce(&DeviceId) -> Option<Secret>,
    ) -> Output<Event> {
        if msg.len() > MAX_FIRST {
            return Output::failed(Error::TooLarge);
        }
        let (hello, at) = match Hello::decode(msg) {
            Ok(decoded) => decoded,
            Err(HelloError::Malformed(e)) => return Output::failed(e),
            Err(HelloError::Version(preamble)) => {
                let theirs = preamble.version;
                return Output {
                    send: vec![Reply::Reject(Reject::Version(VERSION)).encode()],
                    events: vec![Event::Mismatch(preamble)],
                    end: Some(End::Failed(Error::Version {
                        theirs,
                        ours: VERSION,
                    })),
                };
            }
        };
        let (prologue, first) = msg.split_at(at);
        let not_paired = || Output {
            send: vec![Reply::Reject(Reject::NotPaired).encode()],
            events: vec![],
            end: Some(End::Failed(Error::NotPaired)),
        };
        let found = match hello.mode {
            Mode::Pair => None,
            Mode::Link => match secret(&hello.device) {
                Some(found) => Some(found),
                None => return not_paired(),
            },
        };
        let mut hs = crypto::handshake(hello.mode, false, prologue, found.as_ref(), &self.entropy);
        match crypto::read(&mut hs, first) {
            Ok(payload) if payload.is_empty() => {}
            Ok(_) => return Output::failed(Error::Malformed("handshake payload")),
            // Linking: the wrong secret. Unknown and wrong look the same.
            Err(_) if hello.mode == Mode::Link => return not_paired(),
            Err(e) => return Output::failed(e),
        }
        let (nonce, commit) = match hello.mode {
            Mode::Pair => {
                let nonce: [u8; 32] = random(&self.entropy);
                (Some(nonce), Some(crypto::commit(&nonce)))
            }
            Mode::Link => (None, None),
        };
        let payload = wire::encode_desktop(&self.id, &self.name, commit.as_ref());
        let reply = crypto::write(&mut hs, &payload);
        let hash = hs.get_handshake_hash().to_vec();
        let transport = match Transport::new(hs) {
            Ok(t) => t,
            Err(e) => return Output::failed(e),
        };
        self.phase = match nonce {
            Some(nonce) => Phase::Committed {
                transport,
                hello,
                nonce,
                hash,
            },
            None => Phase::Linking { transport, hello },
        };
        self.deadline = now + HANDSHAKE_TIMEOUT_MS;
        Output {
            send: vec![Reply::Handshake(reply).encode()],
            ..Output::default()
        }
    }

    /// Decrypts `msg` in `phase`, which must have a transport.
    fn open(&mut self, mut phase: Phase, msg: &[u8]) -> Result<(Phase, Option<Frame>), Error> {
        // Until the link is up, frames are a few bytes: no big buffers for strangers.
        let limit = match phase {
            Phase::Linking { .. } | Phase::Linked { .. } => MAX_FRAME,
            _ => crate::MAX_PAIRING_FRAME,
        };
        let transport = match &mut phase {
            Phase::Committed { transport, .. }
            | Phase::Comparing { transport, .. }
            | Phase::Accepted { transport }
            | Phase::Linking { transport, .. }
            | Phase::Linked { transport, .. } => transport,
            Phase::Hello | Phase::Ended => unreachable!("handled by receive"),
        };
        let frame = transport.open(msg, limit)?;
        Ok((phase, frame))
    }

    fn frame(&mut self, phase: Phase, frame: Frame, now: u64) -> Output<Event> {
        match (phase, frame) {
            (_, Frame::PairCancel) => Output::failed(Error::Cancelled),
            (
                Phase::Committed {
                    mut transport,
                    hello,
                    nonce,
                    hash,
                },
                Frame::PairNonce(theirs),
            ) => {
                let code = crypto::code(&hash, &theirs, &nonce);
                let send = match transport.seal(&Frame::PairReveal(nonce)) {
                    Ok(send) => send,
                    Err(e) => return Output::failed(e),
                };
                let expires_at = now + PAIR_EXPIRY_MS;
                self.deadline = expires_at;
                self.phase = Phase::Comparing {
                    transport,
                    hello: hello.clone(),
                    allowed: false,
                    confirmed: false,
                };
                Output {
                    send,
                    events: vec![Event::PairRequest {
                        hello,
                        code,
                        expires_at,
                    }],
                    end: None,
                }
            }
            (
                Phase::Comparing {
                    transport,
                    hello,
                    allowed,
                    ..
                },
                Frame::PairConfirm,
            ) => match allowed {
                true => self.accept(transport, hello, now),
                false => {
                    self.phase = Phase::Comparing {
                        transport,
                        hello,
                        allowed,
                        confirmed: true,
                    };
                    Output::default()
                }
            },
            (Phase::Accepted { .. }, Frame::PairDone) => Output {
                end: Some(End::Done),
                ..Output::default()
            },
            (Phase::Linking { transport, hello }, frame) => {
                // The first frame from the device proves it: linked, then
                // the frame itself as on a link.
                self.phase = Phase::Linked { transport };
                let mut out = self.linked_frame(frame, now);
                match out.end {
                    Some(_) => self.phase = Phase::Ended,
                    None => out.events.insert(0, Event::Linked { hello }),
                }
                out
            }
            (phase @ Phase::Linked { .. }, frame) => {
                self.phase = phase;
                let out = self.linked_frame(frame, now);
                if out.end.is_some() {
                    self.phase = Phase::Ended;
                }
                out
            }
            (_, Frame::Close(reason)) => Output::failed(Error::Closed(reason)),
            _ => Output::failed(Error::Unexpected("frame")),
        }
    }

    fn linked_frame(&mut self, frame: Frame, now: u64) -> Output<Event> {
        self.deadline = now + LINK_TIMEOUT_MS;
        let event = match frame {
            Frame::Response(r) => Event::Response(r),
            Frame::Status(s) => Event::Status(s),
            // A pong from before the clock (a bad echo) measures nothing.
            Frame::Pong(t) if t <= now => Event::Latency(now - t),
            Frame::Pong(_) => return Output::default(),
            Frame::Channel { channel, data } => Event::Channel { channel, data },
            Frame::Close(_) => {
                return Output {
                    end: Some(End::Done),
                    ..Output::default()
                }
            }
            // A device answers requests; it never makes them.
            Frame::Request(_) => return Output::failed(Error::Unexpected("request from a device")),
            _ => return Output::failed(Error::Unexpected("frame on a link")),
        };
        Output {
            events: vec![event],
            ..Output::default()
        }
    }

    /// The user allowed the pairing request. Once the headset's user has
    /// confirmed too, a fresh secret goes to the device and [`Event::Paired`]
    /// says to store it.
    pub fn allow(&mut self, now: u64) -> Output<Event> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Comparing {
                transport,
                hello,
                confirmed: true,
                ..
            } => self.accept(transport, hello, now),
            Phase::Comparing {
                transport,
                hello,
                confirmed: false,
                ..
            } => {
                self.phase = Phase::Comparing {
                    transport,
                    hello,
                    allowed: true,
                    confirmed: false,
                };
                Output::default()
            }
            // Not asked now (the peer already went): nothing to do.
            phase => {
                self.phase = phase;
                Output::default()
            }
        }
    }

    /// Both users agreed: send the secret.
    fn accept(&mut self, mut transport: Transport, hello: Hello, now: u64) -> Output<Event> {
        if now >= self.deadline {
            return Output::failed(Error::Expired);
        }
        let secret = Secret(random(&self.entropy));
        let accept = Frame::PairAccept {
            desktop: self.id,
            secret: secret.clone(),
        };
        let send = match transport.seal(&accept) {
            Ok(send) => send,
            Err(e) => return Output::failed(e),
        };
        self.phase = Phase::Accepted { transport };
        self.deadline = now + HANDSHAKE_TIMEOUT_MS;
        Output {
            send,
            events: vec![Event::Paired { hello, secret }],
            end: None,
        }
    }

    /// The user said the code is not theirs.
    pub fn deny(&mut self) -> Output<Event> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Comparing { mut transport, .. } => Output {
                send: transport.seal(&Frame::PairDeny).unwrap_or_default(),
                events: vec![],
                end: Some(End::Failed(Error::Denied)),
            },
            // Not asked now (the peer already went): nothing to do.
            phase => {
                self.phase = phase;
                Output::default()
            }
        }
    }

    /// Sends `frame` on the link: a request, ping, channel or close.
    pub fn send(&mut self, frame: &Frame) -> Result<Vec<Vec<u8>>, Error> {
        let allowed = matches!(
            frame,
            Frame::Request(_) | Frame::Ping(_) | Frame::Channel { .. } | Frame::Close(_)
        );
        match &mut self.phase {
            Phase::Linked { transport, .. } if allowed => transport.seal(frame),
            Phase::Linked { .. } => Err(Error::Unexpected("frame from the desktop")),
            _ => Err(Error::Unexpected("send before the link is up")),
        }
    }

    /// A request to tunnel.
    pub fn request(&mut self, request: Request) -> Result<Vec<Vec<u8>>, Error> {
        self.send(&Frame::Request(request))
    }

    /// Ends the connection if its phase has run out of time at `now`.
    pub fn check(&mut self, now: u64) -> Option<Error> {
        if matches!(self.phase, Phase::Ended) || now < self.deadline {
            return None;
        }
        let error = match self.phase {
            Phase::Comparing { .. } => Error::Expired,
            _ => Error::Timeout,
        };
        self.phase = Phase::Ended;
        Some(error)
    }
}
