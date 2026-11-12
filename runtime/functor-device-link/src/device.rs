//! The device's side of one connection: it says hello, then pairs (showing
//! the code until the desktop's user decides) or links with its secret and
//! serves the desktop's requests.

use crate::crypto::{self, Transport};
use crate::wire::{self, Frame, Hello, Mode, Reject, Reply, Request, Response, Status};
use crate::{random, DesktopId, DeviceId, End, Error, Output, Secret, SharedEntropy};
use crate::{HANDSHAKE_TIMEOUT_MS, LINK_TIMEOUT_MS, PAIR_EXPIRY_MS, VERSION};

/// Who the device is, as its hello says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub device: DeviceId,
    pub name: String,
    pub app_version: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// Show `code` until `expires_at` and ask the user whether the desktop
    /// shows the same one: [`Device::confirm`] if it does, else
    /// [`Device::cancel`]. Nothing is stored until both users agree.
    Code {
        desktop: DesktopId,
        desktop_name: String,
        code: String,
        expires_at: u64,
    },
    /// The desktop's user allowed it: store `secret` for `desktop`. The
    /// connection then ends; link with [`Device::link`].
    Paired {
        desktop: DesktopId,
        desktop_name: String,
        secret: Secret,
    },
    /// The link is up.
    Linked {
        desktop: DesktopId,
        desktop_name: String,
    },
    /// Answer with [`Device::respond`].
    Request(Request),
    Channel {
        channel: u16,
        data: Vec<u8>,
    },
    /// The desktop refused in the clear. Report it, change nothing stored:
    /// anyone on the network can forge one.
    Rejected(Reject),
}

enum Phase {
    /// Our hello and first handshake message are sent.
    Sent {
        hs: Box<snow::HandshakeState>,
        mode: Mode,
    },
    /// Pairing: we sent our nonce; the desktop reveals the one it committed to.
    Committed {
        transport: Transport,
        desktop: (DesktopId, String),
        hash: Vec<u8>,
        nonce: [u8; 32],
        commit: [u8; 32],
    },
    Comparing {
        transport: Transport,
        desktop: (DesktopId, String),
        /// Our user said the codes match.
        confirmed: bool,
    },
    Linked {
        transport: Transport,
    },
    Ended,
}

pub struct Device {
    phase: Phase,
    deadline: u64,
    /// Sent as the first transport message when linking.
    status: Status,
    entropy: SharedEntropy,
}

impl Device {
    /// Starts pairing: the output sends the hello.
    pub fn pair(identity: &Identity, entropy: SharedEntropy, now: u64) -> (Device, Output<Event>) {
        Self::start(identity, Mode::Pair, None, Status::default(), entropy, now)
    }

    /// Starts a link with the secret from pairing; `status` is the first
    /// thing the desktop hears.
    pub fn link(
        identity: &Identity,
        secret: &Secret,
        status: Status,
        entropy: SharedEntropy,
        now: u64,
    ) -> (Device, Output<Event>) {
        Self::start(identity, Mode::Link, Some(secret), status, entropy, now)
    }

    fn start(
        identity: &Identity,
        mode: Mode,
        secret: Option<&Secret>,
        status: Status,
        entropy: SharedEntropy,
        now: u64,
    ) -> (Device, Output<Event>) {
        let hello = Hello {
            version: VERSION,
            device: identity.device,
            name: identity.name.clone(),
            mode,
            app_version: identity.app_version.clone(),
        }
        .encode();
        let mut hs = crypto::handshake(mode, true, &hello, secret, &entropy);
        let mut first = hello;
        first.extend(crypto::write(&mut hs, &[]));
        let device = Device {
            phase: Phase::Sent {
                hs: Box::new(hs),
                mode,
            },
            deadline: now + HANDSHAKE_TIMEOUT_MS,
            status,
            entropy,
        };
        let out = Output {
            send: vec![first],
            ..Output::default()
        };
        (device, out)
    }

    pub fn linked(&self) -> bool {
        matches!(self.phase, Phase::Linked { .. })
    }

    /// One received WebSocket message.
    pub fn receive(&mut self, msg: &[u8], now: u64) -> Output<Event> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Sent { hs, mode } => self.reply(*hs, mode, msg, now),
            Phase::Ended => Output::failed(Error::Ended),
            mut phase => {
                // Until the link is up, frames are a few bytes.
                let limit = match phase {
                    Phase::Linked { .. } => crate::MAX_FRAME,
                    _ => crate::MAX_PAIRING_FRAME,
                };
                let transport = match &mut phase {
                    Phase::Committed { transport, .. }
                    | Phase::Comparing { transport, .. }
                    | Phase::Linked { transport } => transport,
                    Phase::Sent { .. } | Phase::Ended => unreachable!("matched above"),
                };
                match transport.open(msg, limit) {
                    Ok(None) => {
                        self.phase = phase;
                        Output::default()
                    }
                    Ok(Some(frame)) => self.frame(phase, frame, now),
                    Err(e) => Output::failed(e),
                }
            }
        }
    }

    fn reply(
        &mut self,
        mut hs: snow::HandshakeState,
        mode: Mode,
        msg: &[u8],
        now: u64,
    ) -> Output<Event> {
        let theirs = match Reply::decode(msg) {
            Ok(Reply::Handshake(theirs)) => theirs,
            Ok(Reply::Reject(reject)) => {
                let error = match reject {
                    Reject::Version(theirs) => Error::Version {
                        theirs,
                        ours: VERSION,
                    },
                    Reject::NotPaired => Error::NotPaired,
                    Reject::Busy => Error::Busy,
                };
                return Output {
                    events: vec![Event::Rejected(reject)],
                    end: Some(End::Failed(error)),
                    ..Output::default()
                };
            }
            Err(e) => return Output::failed(e),
        };
        // Linking: a desktop without our secret fails here.
        let payload = match crypto::read(&mut hs, &theirs) {
            Ok(payload) => payload,
            Err(e) => return Output::failed(e),
        };
        let (id, name, commit) = match wire::decode_desktop(&payload, mode == Mode::Pair) {
            Ok(fields) => fields,
            Err(e) => return Output::failed(e),
        };
        let hash = hs.get_handshake_hash().to_vec();
        let mut transport = match Transport::new(hs) {
            Ok(t) => t,
            Err(e) => return Output::failed(e),
        };
        self.deadline = now + HANDSHAKE_TIMEOUT_MS;
        match commit {
            Some(commit) => {
                let nonce: [u8; 32] = random(&self.entropy);
                let send = match transport.seal(&Frame::PairNonce(nonce)) {
                    Ok(send) => send,
                    Err(e) => return Output::failed(e),
                };
                self.phase = Phase::Committed {
                    transport,
                    desktop: (id, name),
                    hash,
                    nonce,
                    commit,
                };
                Output {
                    send,
                    ..Output::default()
                }
            }
            None => {
                let send = match transport.seal(&Frame::Status(self.status.clone())) {
                    Ok(send) => send,
                    Err(e) => return Output::failed(e),
                };
                self.phase = Phase::Linked { transport };
                self.deadline = now + LINK_TIMEOUT_MS;
                Output {
                    send,
                    events: vec![Event::Linked {
                        desktop: id,
                        desktop_name: name,
                    }],
                    end: None,
                }
            }
        }
    }

    fn frame(&mut self, phase: Phase, frame: Frame, now: u64) -> Output<Event> {
        match (phase, frame) {
            (_, Frame::PairCancel) => Output::failed(Error::Cancelled),
            (
                Phase::Committed {
                    transport,
                    desktop,
                    hash,
                    nonce,
                    commit,
                },
                Frame::PairReveal(theirs),
            ) => {
                // Not the nonce it committed to before seeing ours.
                if crypto::commit(&theirs) != commit {
                    return Output::failed(Error::Crypto);
                }
                let code = crypto::code(&hash, &nonce, &theirs);
                let expires_at = now + PAIR_EXPIRY_MS;
                self.deadline = expires_at;
                let event = Event::Code {
                    desktop: desktop.0,
                    desktop_name: desktop.1.clone(),
                    code,
                    expires_at,
                };
                self.phase = Phase::Comparing {
                    transport,
                    desktop,
                    confirmed: false,
                };
                Output {
                    events: vec![event],
                    ..Output::default()
                }
            }
            // Only once our user confirmed: a desktop that answers its own
            // prompt at once cannot pair a headset nobody looked at.
            (
                Phase::Comparing {
                    mut transport,
                    desktop,
                    confirmed: true,
                },
                Frame::PairAccept {
                    desktop: id,
                    secret,
                },
            ) if id == desktop.0 => Output {
                send: transport.seal(&Frame::PairDone).unwrap_or_default(),
                events: vec![Event::Paired {
                    desktop: id,
                    desktop_name: desktop.1,
                    secret,
                }],
                end: Some(End::Done),
            },
            (Phase::Comparing { .. }, Frame::PairDeny) => Output::failed(Error::Denied),
            (Phase::Linked { mut transport }, frame) => {
                self.deadline = now + LINK_TIMEOUT_MS;
                let mut out = Output::default();
                match frame {
                    Frame::Request(r) => out.events.push(Event::Request(r)),
                    Frame::Ping(t) => match transport.seal(&Frame::Pong(t)) {
                        Ok(send) => out.send = send,
                        Err(e) => return Output::failed(e),
                    },
                    Frame::Channel { channel, data } => {
                        out.events.push(Event::Channel { channel, data })
                    }
                    Frame::Close(_) => {
                        out.end = Some(End::Done);
                        return out;
                    }
                    _ => return Output::failed(Error::Unexpected("frame on a link")),
                }
                self.phase = Phase::Linked { transport };
                out
            }
            (_, Frame::Close(reason)) => Output::failed(Error::Closed(reason)),
            _ => Output::failed(Error::Unexpected("frame")),
        }
    }

    /// Sends `frame` on the link: a response, status, channel or close.
    pub fn send(&mut self, frame: &Frame) -> Result<Vec<Vec<u8>>, Error> {
        let allowed = matches!(
            frame,
            Frame::Response(_) | Frame::Status(_) | Frame::Channel { .. } | Frame::Close(_)
        );
        match &mut self.phase {
            Phase::Linked { transport } if allowed => transport.seal(frame),
            Phase::Linked { .. } => Err(Error::Unexpected("frame from the device")),
            _ => Err(Error::Unexpected("send before the link is up")),
        }
    }

    /// Any frame, as a misbehaving device would send it.
    #[cfg(test)]
    pub(crate) fn send_raw(&mut self, frame: &Frame) -> Vec<Vec<u8>> {
        match &mut self.phase {
            Phase::Linked { transport } => transport.seal(frame).unwrap(),
            _ => panic!("not linked"),
        }
    }

    /// Answers a request.
    pub fn respond(&mut self, response: Response) -> Result<Vec<Vec<u8>>, Error> {
        self.send(&Frame::Response(response))
    }

    /// The user says the desktop shows the same code.
    pub fn confirm(&mut self) -> Output<Event> {
        match &mut self.phase {
            Phase::Comparing {
                transport,
                confirmed: confirmed @ false,
                ..
            } => match transport.seal(&Frame::PairConfirm) {
                Ok(send) => {
                    *confirmed = true;
                    Output {
                        send,
                        ..Output::default()
                    }
                }
                Err(e) => {
                    self.phase = Phase::Ended;
                    Output::failed(e)
                }
            },
            _ => Output::default(),
        }
    }

    /// Stops pairing (the user backed out): the desktop is told.
    pub fn cancel(&mut self) -> Output<Event> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Committed { mut transport, .. } | Phase::Comparing { mut transport, .. } => {
                Output {
                    send: transport.seal(&Frame::PairCancel).unwrap_or_default(),
                    events: vec![],
                    end: Some(End::Failed(Error::Cancelled)),
                }
            }
            // Not asked now (the peer already went): nothing to do.
            phase => {
                self.phase = phase;
                Output::default()
            }
        }
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
