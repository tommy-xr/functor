//! Byte layouts: the clear hello and reply, and the frames carried inside
//! the encrypted transport. Big-endian; strings are UTF-8 with a length
//! prefix; every length is checked against its limit before use.

use crate::{DesktopId, DeviceId, Error, Secret, MAX_FRAME};

/// The first four bytes of every hello.
pub const MAGIC: [u8; 4] = *b"FLNK";

pub const MAX_NAME: usize = 64;
pub const MAX_APP_VERSION: usize = 32;
pub const MAX_METHOD: usize = 16;
pub const MAX_TARGET: usize = 8 * 1024;
pub const MAX_HEADERS: usize = 32;
pub const MAX_HEADER_NAME: usize = 64;
pub const MAX_HEADER_VALUE: usize = 4 * 1024;
pub const MAX_BUILD: usize = 128;
pub const MAX_PROJECT: usize = 1024;
pub const MAX_REASON: usize = 256;

/// What a connection is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Not paired yet: pair, then close.
    Pair,
    /// Paired: authenticate with the secret and carry the link.
    Link,
}

/// The device's first message, in the clear, before its handshake message.
///
/// The PREAMBLE — magic, version, device id, name — keeps this layout in
/// every version, so a desktop can name a device whose version it does not
/// speak. The rest may change with the version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub version: u16,
    pub device: DeviceId,
    /// For people: "Quest 3". No control characters.
    pub name: String,
    pub mode: Mode,
    /// The headset app's version, `major.minor.patch`.
    pub app_version: String,
}

impl Hello {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(&MAGIC);
        w.u16(self.version);
        w.bytes(&self.device);
        w.str8(clip(&self.name, MAX_NAME));
        w.u8(match self.mode {
            Mode::Pair => 0,
            Mode::Link => 1,
        });
        w.str8(clip(&self.app_version, MAX_APP_VERSION));
        w.0
    }

    /// The hello at the start of `msg`, and where it ends. A version other
    /// than ours is `Err(Version)` with the preamble's fields: see
    /// [`Preamble`].
    pub fn decode(msg: &[u8]) -> Result<(Hello, usize), HelloError> {
        let mut r = Reader::new(msg);
        let preamble = Preamble::read(&mut r).map_err(HelloError::Malformed)?;
        if preamble.version != crate::VERSION {
            return Err(HelloError::Version(preamble));
        }
        let mut rest = || -> Result<(Mode, String), Error> {
            let mode = match r.u8()? {
                0 => Mode::Pair,
                1 => Mode::Link,
                _ => return Err(Error::Malformed("hello mode")),
            };
            let app_version = r.text8(MAX_APP_VERSION)?;
            Ok((mode, app_version))
        };
        let (mode, app_version) = rest().map_err(HelloError::Malformed)?;
        let hello = Hello {
            version: preamble.version,
            device: preamble.device,
            name: preamble.name,
            mode,
            app_version,
        };
        Ok((hello, r.at))
    }
}

/// The fields every version's hello starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preamble {
    pub version: u16,
    pub device: DeviceId,
    pub name: String,
}

impl Preamble {
    fn read(r: &mut Reader) -> Result<Preamble, Error> {
        if r.take(4)? != MAGIC {
            return Err(Error::Malformed("hello magic"));
        }
        Ok(Preamble {
            version: r.u16()?,
            device: r.array()?,
            name: r.text8(MAX_NAME)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelloError {
    Malformed(Error),
    Version(Preamble),
}

/// The desktop's answer to a hello: its handshake message, or why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Handshake(Vec<u8>),
    Reject(Reject),
}

/// Why the desktop will not go on. In the clear, so a device must not act
/// on one beyond reporting it (a machine on the network can forge it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The desktop speaks this version.
    Version(u16),
    NotPaired,
    Busy,
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Reply::Handshake(msg) => {
                w.u8(0);
                w.bytes(msg);
            }
            Reply::Reject(Reject::Version(v)) => {
                w.u8(1);
                w.u8(1);
                w.u16(*v);
            }
            Reply::Reject(Reject::NotPaired) => {
                w.u8(1);
                w.u8(2);
            }
            Reply::Reject(Reject::Busy) => {
                w.u8(1);
                w.u8(3);
            }
        }
        w.0
    }

    pub fn decode(msg: &[u8]) -> Result<Reply, Error> {
        let mut r = Reader::new(msg);
        let reply = match r.u8()? {
            0 => return Ok(Reply::Handshake(msg[1..].to_vec())),
            1 => match r.u8()? {
                1 => Reply::Reject(Reject::Version(r.u16()?)),
                2 => Reply::Reject(Reject::NotPaired),
                3 => Reply::Reject(Reject::Busy),
                _ => return Err(Error::Malformed("reject reason")),
            },
            _ => return Err(Error::Malformed("reply")),
        };
        r.end()?;
        Ok(reply)
    }
}

/// A debug-runtime HTTP request, tunnelled desktop → device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The desktop's; the response repeats it.
    pub id: u32,
    pub method: String,
    /// Path and query: `/state`, `/frames?count=3`.
    pub target: String,
    /// The subset the debug runtime reads (content type, origin, accept).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Its answer, device → desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub id: u32,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The device's state, sent when the link comes up and every heartbeat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Percent; `None` when unknown.
    pub battery: Option<u8>,
    pub charging: bool,
    /// The runtime build the headset runs.
    pub build: String,
    /// The project it runs, if any.
    pub project: Option<String>,
}

/// One message inside the encrypted transport.
///
/// New kinds of traffic (audio clips, capsules, panel state) travel as
/// [`Frame::Channel`] with their own channel number, so adding one changes
/// neither this enum nor the version; a side drops channels it does not
/// know. Streamed bodies will be new frames under a new version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Desktop → device.
    Request(Request),
    /// Device → desktop.
    Response(Response),
    /// Device → desktop.
    Status(Status),
    /// Desktop → device, carrying the desktop's clock; echoed as `Pong`.
    Ping(u64),
    Pong(u64),
    /// Either way: an opaque payload on a typed channel.
    Channel {
        channel: u16,
        data: Vec<u8>,
    },
    /// Either way: the sender is closing, saying why.
    Close(String),
    /// Pairing, device → desktop: its nonce, after the desktop committed.
    PairNonce([u8; 32]),
    /// Pairing, desktop → device: the nonce it committed to.
    PairReveal([u8; 32]),
    /// Pairing, desktop → device: the user allowed it.
    PairAccept {
        desktop: DesktopId,
        secret: Secret,
    },
    /// Pairing, device → desktop: the secret is stored.
    PairDone,
    /// Pairing, desktop → device: the user said the code is not theirs.
    PairDeny,
    /// Pairing, either way: no longer pairing.
    PairCancel,
    /// Pairing, device → desktop: its user says the codes match.
    PairConfirm,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Frame::Request(req) => {
                w.u8(1);
                w.u32(req.id);
                w.str16(&req.method);
                w.str16(&req.target);
                w.headers(&req.headers);
                w.blob(&req.body);
            }
            Frame::Response(res) => {
                w.u8(2);
                w.u32(res.id);
                w.u16(res.status);
                w.headers(&res.headers);
                w.blob(&res.body);
            }
            Frame::Status(s) => {
                w.u8(3);
                w.u8(s.battery.unwrap_or(u8::MAX));
                w.u8(u8::from(s.charging));
                w.str16(&s.build);
                match &s.project {
                    Some(p) => {
                        w.u8(1);
                        w.str16(p);
                    }
                    None => w.u8(0),
                }
            }
            Frame::Ping(t) => {
                w.u8(4);
                w.u64(*t);
            }
            Frame::Pong(t) => {
                w.u8(5);
                w.u64(*t);
            }
            Frame::Channel { channel, data } => {
                w.u8(6);
                w.u16(*channel);
                w.blob(data);
            }
            Frame::Close(reason) => {
                w.u8(7);
                w.str16(reason);
            }
            Frame::PairNonce(n) => {
                w.u8(16);
                w.bytes(n);
            }
            Frame::PairReveal(n) => {
                w.u8(17);
                w.bytes(n);
            }
            Frame::PairAccept { desktop, secret } => {
                w.u8(18);
                w.bytes(desktop);
                w.bytes(&secret.0);
            }
            Frame::PairDone => w.u8(19),
            Frame::PairDeny => w.u8(20),
            Frame::PairCancel => w.u8(21),
            Frame::PairConfirm => w.u8(22),
        }
        w.0
    }

    /// Whether `decode` would take this frame back: every text within its
    /// limit and allowed. Checked before sending, so a frame the peer would
    /// refuse is an error here, not a dropped link there.
    pub fn check(&self) -> Result<(), Error> {
        let text = |t: &str, max: usize| match () {
            _ if t.len() > max => Err(Error::TooLarge),
            _ if !allowed(t) => Err(Error::Malformed("text")),
            _ => Ok(()),
        };
        let headers = |hs: &[(String, String)]| {
            if hs.len() > MAX_HEADERS {
                return Err(Error::TooLarge);
            }
            hs.iter().try_for_each(|(n, v)| {
                text(n, MAX_HEADER_NAME)?;
                text(v, MAX_HEADER_VALUE)
            })
        };
        match self {
            Frame::Request(r) => {
                text(&r.method, MAX_METHOD)?;
                text(&r.target, MAX_TARGET)?;
                headers(&r.headers)
            }
            Frame::Response(r) => headers(&r.headers),
            Frame::Status(s) => {
                if s.battery.is_some_and(|b| b > 100) {
                    return Err(Error::Malformed("battery"));
                }
                text(&s.build, MAX_BUILD)?;
                s.project
                    .as_deref()
                    .map_or(Ok(()), |p| text(p, MAX_PROJECT))
            }
            Frame::Close(reason) => text(reason, MAX_REASON),
            _ => Ok(()),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, Error> {
        if bytes.len() > MAX_FRAME {
            return Err(Error::TooLarge);
        }
        let mut r = Reader::new(bytes);
        let frame = match r.u8()? {
            1 => Frame::Request(Request {
                id: r.u32()?,
                method: r.text16(MAX_METHOD)?,
                target: r.text16(MAX_TARGET)?,
                headers: r.headers()?,
                body: r.blob()?,
            }),
            2 => Frame::Response(Response {
                id: r.u32()?,
                status: r.u16()?,
                headers: r.headers()?,
                body: r.blob()?,
            }),
            3 => Frame::Status(Status {
                battery: match r.u8()? {
                    u8::MAX => None,
                    b if b <= 100 => Some(b),
                    _ => return Err(Error::Malformed("battery")),
                },
                charging: r.flag()?,
                build: r.text16(MAX_BUILD)?,
                project: match r.flag()? {
                    true => Some(r.text16(MAX_PROJECT)?),
                    false => None,
                },
            }),
            4 => Frame::Ping(r.u64()?),
            5 => Frame::Pong(r.u64()?),
            6 => Frame::Channel {
                channel: r.u16()?,
                data: r.blob()?,
            },
            7 => Frame::Close(r.text16(MAX_REASON)?),
            16 => Frame::PairNonce(r.array()?),
            17 => Frame::PairReveal(r.array()?),
            18 => Frame::PairAccept {
                desktop: r.array()?,
                secret: Secret(r.array()?),
            },
            19 => Frame::PairDone,
            20 => Frame::PairDeny,
            21 => Frame::PairCancel,
            22 => Frame::PairConfirm,
            _ => return Err(Error::Malformed("frame kind")),
        };
        r.end()?;
        Ok(frame)
    }
}

/// The desktop's handshake payload: who it is, and when pairing, its
/// commitment to its nonce.
pub(crate) fn encode_desktop(id: &DesktopId, name: &str, commit: Option<&[u8; 32]>) -> Vec<u8> {
    let mut w = Writer::default();
    w.bytes(id);
    w.str8(clip(name, MAX_NAME));
    if let Some(c) = commit {
        w.bytes(c);
    }
    w.0
}

pub(crate) fn decode_desktop(
    payload: &[u8],
    pairing: bool,
) -> Result<(DesktopId, String, Option<[u8; 32]>), Error> {
    let mut r = Reader::new(payload);
    let id = r.array()?;
    let name = r.text8(MAX_NAME)?;
    let commit = if pairing { Some(r.array()?) } else { None };
    r.end()?;
    Ok((id, name, commit))
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    /// Callers keep these within their limits; longer is cut, never wrapped.
    fn str8(&mut self, s: &str) {
        let s = clip(s, u8::MAX as usize);
        self.u8(s.len() as u8);
        self.bytes(s.as_bytes());
    }
    fn str16(&mut self, s: &str) {
        let s = clip(s, u16::MAX as usize);
        self.u16(s.len() as u16);
        self.bytes(s.as_bytes());
    }
    fn blob(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.bytes(v);
    }
    fn headers(&mut self, headers: &[(String, String)]) {
        let headers = &headers[..headers.len().min(MAX_HEADERS)];
        self.u8(headers.len() as u8);
        for (name, value) in headers {
            self.str16(name);
            self.str16(value);
        }
    }
}

/// Text a peer may send: no control characters, and no invisible ones
/// that reorder or hide text (a name that reads like another's).
fn allowed(text: &str) -> bool {
    !text.chars().any(|c| {
        c.is_control()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
    })
}

/// `s` cut to at most `max` bytes, on a character boundary.
fn clip(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len());
        let end = end.ok_or(Error::Malformed("truncated"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn flag(&mut self) -> Result<bool, Error> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Malformed("flag")),
        }
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn text(&mut self, len: usize, max: usize) -> Result<String, Error> {
        if len > max {
            return Err(Error::TooLarge);
        }
        let text = std::str::from_utf8(self.take(len)?).map_err(|_| Error::Malformed("text"))?;
        if !allowed(text) {
            return Err(Error::Malformed("text"));
        }
        Ok(text.to_string())
    }
    fn text8(&mut self, max: usize) -> Result<String, Error> {
        let len = self.u8()? as usize;
        self.text(len, max)
    }
    fn text16(&mut self, max: usize) -> Result<String, Error> {
        let len = self.u16()? as usize;
        self.text(len, max)
    }
    fn blob(&mut self) -> Result<Vec<u8>, Error> {
        let len = self.u32()? as usize;
        if len > MAX_FRAME {
            return Err(Error::TooLarge);
        }
        Ok(self.take(len)?.to_vec())
    }
    fn headers(&mut self) -> Result<Vec<(String, String)>, Error> {
        let n = self.u8()? as usize;
        if n > MAX_HEADERS {
            return Err(Error::TooLarge);
        }
        (0..n)
            .map(|_| {
                Ok((
                    self.text16(MAX_HEADER_NAME)?,
                    self.text16(MAX_HEADER_VALUE)?,
                ))
            })
            .collect()
    }
    fn end(&self) -> Result<(), Error> {
        match self.at == self.bytes.len() {
            true => Ok(()),
            false => Err(Error::Malformed("trailing bytes")),
        }
    }
}
