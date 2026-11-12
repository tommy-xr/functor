//! The device link: one WebSocket a headset opens to the desktop app. It
//! carries tunnelled debug-runtime requests (desktop → device) and their
//! responses and the device's status (device → desktop).
//!
//! Sans-IO: [`desktop::Desktop`] and [`device::Device`] take each received
//! WebSocket binary message and return the messages to send and what
//! happened; the shells own sockets, clocks (milliseconds, any epoch) and
//! storage.
//!
//! On the wire, the device's first message is a clear [`wire::Hello`]
//! followed by a Noise handshake message whose prologue is that hello, so
//! nobody can alter it unnoticed. The desktop answers with a [`wire::Reply`].
//!
//! - **Pairing** (`Noise_NN`): unauthenticated key agreement, then a
//!   commit-reveal of one random nonce per side, then both show a code
//!   derived from the handshake hash and both nonces ([`code`]). A machine in
//!   the middle runs two handshakes with different hashes and must fix its
//!   nonce in each before learning the other side's, so it cannot steer the
//!   two codes to match (a one-in-[`CODE_SPACE`] chance per attempt). Only
//!   after the user allows it does the desktop send a fresh pairing secret.
//! - **Linking** (`Noise_NNpsk0`, the pairing secret as the PSK): mutual
//!   authentication with fresh ephemeral keys. The desktop counts the device
//!   as linked only once its first transport message decrypts, so a replayed
//!   handshake message never links.
//!
//! After the handshake every message is a Noise transport message: AEAD
//! with an implicit counter, so a tampered, replayed, reordered or dropped
//! message fails to decrypt and ends the connection.

pub mod desktop;
pub mod device;
pub mod wire;

mod crypto;

use std::fmt;
use std::sync::{Arc, Mutex};

pub use crypto::code;

/// The link protocol's version, in every hello. Peers of other versions are
/// refused with [`wire::Reject::Version`]; neither side guesses.
pub const VERSION: u16 = 1;

/// Largest WebSocket message either side sends or accepts: one Noise
/// message.
pub const MAX_MESSAGE: usize = 65_535;

/// Largest frame once reassembled from its messages: a request or response
/// with its whole body. Larger bodies need the streaming frames a later
/// version adds.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Largest frame before the link is up: pairing frames are under 64 bytes.
pub const MAX_PAIRING_FRAME: usize = 256;

/// Digits in the pairing code: see [`code`].
pub const CODE_DIGITS: usize = 6;
/// How many pairing codes there are.
pub const CODE_SPACE: u64 = 1_000_000;

/// How long a pairing code stays good once both sides show it.
pub const PAIR_EXPIRY_MS: u64 = 120_000;
/// How long either side waits for the other's next handshake message.
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;
/// How often the device sends its status and the desktop pings.
pub const HEARTBEAT_MS: u64 = 5_000;
/// A linked peer silent this long is gone.
pub const LINK_TIMEOUT_MS: u64 = 15_000;

/// A device's id: random, made once per install.
pub type DeviceId = [u8; 16];
/// A desktop's id: random, made once per app home.
pub type DesktopId = [u8; 16];

/// A pairing secret. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub [u8; 32]);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Secret {
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    pub fn from_hex(text: &str) -> Option<Secret> {
        unhex(text).map(Secret)
    }
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `N` bytes from exactly `2 * N` hex digits.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != 2 * N {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0; N];
    for (i, pair) in text.chunks(2).enumerate() {
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// Where keys, nonces and secrets come from: [`OsEntropy`] in the shells, a
/// seeded one in tests.
pub trait Entropy: Send {
    fn fill(&mut self, out: &mut [u8]);
}

/// The operating system's randomness.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&mut self, out: &mut [u8]) {
        getrandom::fill(out).expect("the OS has randomness");
    }
}

/// One `Entropy` for a connection's machine and the Noise handshake it runs.
pub type SharedEntropy = Arc<Mutex<dyn Entropy>>;

pub fn shared(entropy: impl Entropy + 'static) -> SharedEntropy {
    Arc::new(Mutex::new(entropy))
}

fn random<const N: usize>(entropy: &SharedEntropy) -> [u8; N] {
    let mut out = [0; N];
    entropy.lock().unwrap().fill(&mut out);
    out
}

/// Why a connection ended badly. Each machine closes on the first one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Bytes that do not parse.
    Malformed(&'static str),
    /// A message valid on its own but not here: a frame of the wrong
    /// direction or phase, a second hello.
    Unexpected(&'static str),
    /// A message or frame over its limit.
    TooLarge,
    /// The peer speaks another protocol version.
    Version { theirs: u16, ours: u16 },
    /// A handshake or transport message failed to decrypt: tampered,
    /// replayed, out of order, or (linking) the wrong secret.
    Crypto,
    /// The desktop does not know this device or its secret.
    NotPaired,
    /// The desktop has too many unpaired connections.
    Busy,
    /// The pairing code expired before it was allowed.
    Expired,
    /// The desktop's user said the code was not theirs.
    Denied,
    /// The other side cancelled the pairing.
    Cancelled,
    /// The peer did not answer in time.
    Timeout,
    /// The peer closed the link, saying why.
    Closed(String),
    /// Input after the machine closed.
    Ended,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Malformed(what) => write!(f, "malformed {what}"),
            Error::Unexpected(what) => write!(f, "unexpected {what}"),
            Error::TooLarge => f.write_str("message too large"),
            Error::Version { theirs, ours } => {
                write!(f, "link protocol {theirs}, this side speaks {ours}")
            }
            Error::Crypto => f.write_str("a message failed authentication"),
            Error::NotPaired => f.write_str("not paired with this desktop"),
            Error::Busy => f.write_str("the desktop is busy"),
            Error::Expired => f.write_str("the pairing code expired"),
            Error::Denied => f.write_str("pairing was refused"),
            Error::Cancelled => f.write_str("pairing was cancelled"),
            Error::Timeout => f.write_str("timed out"),
            Error::Closed(reason) => write!(f, "closed by the peer: {reason}"),
            Error::Ended => f.write_str("the link has ended"),
        }
    }
}

impl std::error::Error for Error {}

/// How a machine's connection ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// As it should: a pairing finished, or a clean close.
    Done,
    Failed(Error),
}

/// What one call on a machine produced: messages to send in order, what
/// happened, and whether the connection is now over (send first, then
/// close).
#[derive(Debug)]
pub struct Output<E> {
    pub send: Vec<Vec<u8>>,
    pub events: Vec<E>,
    pub end: Option<End>,
}

impl<E> Default for Output<E> {
    fn default() -> Self {
        Self {
            send: Vec::new(),
            events: Vec::new(),
            end: None,
        }
    }
}

impl<E> Output<E> {
    fn failed(error: Error) -> Self {
        Self {
            end: Some(End::Failed(error)),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests;
