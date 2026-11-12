//! Noise handshakes with injected randomness, the pairing code, and the
//! transport that chunks frames into Noise messages.

use blake2::{Blake2s256, Digest};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use snow::types::{Cipher, Dh, Hash, Random};
use snow::{HandshakeState, TransportState};

use crate::wire::{Frame, Mode};
use crate::{Error, Secret, SharedEntropy, CODE_DIGITS, CODE_SPACE, MAX_FRAME, MAX_MESSAGE};

const PAIR: &str = "Noise_NN_25519_ChaChaPoly_BLAKE2s";
const LINK: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
/// Noise's AEAD tag.
const TAG: usize = 16;
/// Plaintext per transport message: one flag byte, then the chunk.
const CHUNK: usize = MAX_MESSAGE - TAG - 1;

/// snow's default crypto with our randomness.
struct Resolver(SharedEntropy);

struct Rng(SharedEntropy);

impl Random for Rng {
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), snow::Error> {
        self.0.lock().unwrap().fill(out);
        Ok(())
    }
}

impl CryptoResolver for Resolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(Rng(self.0.clone())))
    }
    fn resolve_dh(&self, choice: &snow::params::DHChoice) -> Option<Box<dyn Dh>> {
        DefaultResolver.resolve_dh(choice)
    }
    fn resolve_hash(&self, choice: &snow::params::HashChoice) -> Option<Box<dyn Hash>> {
        DefaultResolver.resolve_hash(choice)
    }
    fn resolve_cipher(&self, choice: &snow::params::CipherChoice) -> Option<Box<dyn Cipher>> {
        DefaultResolver.resolve_cipher(choice)
    }
}

/// A handshake for `mode`, bound to `prologue` (the hello); linking needs
/// the pairing secret.
pub(crate) fn handshake(
    mode: Mode,
    initiator: bool,
    prologue: &[u8],
    secret: Option<&Secret>,
    entropy: &SharedEntropy,
) -> HandshakeState {
    let pattern = match mode {
        Mode::Pair => PAIR,
        Mode::Link => LINK,
    };
    let params = pattern.parse().expect("a valid pattern");
    let builder = snow::Builder::with_resolver(params, Box::new(Resolver(entropy.clone())))
        .prologue(prologue)
        .expect("one prologue");
    let builder = match (mode, secret) {
        (Mode::Link, Some(secret)) => builder.psk(0, &secret.0).expect("psk slot 0"),
        _ => builder,
    };
    match initiator {
        true => builder.build_initiator(),
        false => builder.build_responder(),
    }
    .expect("the handshake builds")
}

/// Writes the next handshake message carrying `payload`.
pub(crate) fn write(hs: &mut HandshakeState, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0; MAX_MESSAGE];
    let n = hs
        .write_message(payload, &mut out)
        .expect("a small payload");
    out.truncate(n);
    out
}

/// Reads the next handshake message, giving its payload.
pub(crate) fn read(hs: &mut HandshakeState, msg: &[u8]) -> Result<Vec<u8>, Error> {
    if msg.len() > MAX_MESSAGE {
        return Err(Error::TooLarge);
    }
    let mut out = vec![0; MAX_MESSAGE];
    let n = hs.read_message(msg, &mut out).map_err(|_| Error::Crypto)?;
    out.truncate(n);
    Ok(out)
}

/// The desktop's commitment to its pairing nonce.
pub(crate) fn commit(nonce: &[u8; 32]) -> [u8; 32] {
    Blake2s256::new()
        .chain_update(b"functor-link commit v1")
        .chain_update(nonce)
        .finalize()
        .into()
}

/// The pairing code both sides show: [`CODE_DIGITS`] decimal digits from
/// the handshake hash (which binds both ephemeral keys) and both nonces.
///
/// Digits, because the code is read, not typed: they are legible at a
/// headset's resolution, have no look-alikes, and can be read aloud. Six
/// give an attacker in the middle one chance in a million per attempt, and
/// every attempt puts a prompt in front of the desktop's user.
pub fn code(handshake_hash: &[u8], device_nonce: &[u8; 32], desktop_nonce: &[u8; 32]) -> String {
    let digest: [u8; 32] = Blake2s256::new()
        .chain_update(b"functor-link code v1")
        .chain_update(handshake_hash)
        .chain_update(device_nonce)
        .chain_update(desktop_nonce)
        .finalize()
        .into();
    // A 64-bit draw: the modulo's bias is below 2^-44.
    let n = u64::from_be_bytes(digest[..8].try_into().unwrap()) % CODE_SPACE;
    format!("{n:0width$}", width = CODE_DIGITS)
}

/// The encrypted transport: each frame is cut into chunks of one Noise
/// message each, a flag byte saying whether more follow.
pub(crate) struct Transport {
    noise: TransportState,
    partial: Vec<u8>,
}

impl Transport {
    pub(crate) fn new(hs: HandshakeState) -> Result<Transport, Error> {
        let noise = hs.into_transport_mode().map_err(|_| Error::Crypto)?;
        Ok(Transport {
            noise,
            partial: Vec::new(),
        })
    }

    /// The messages carrying `frame`; a frame over [`MAX_FRAME`] is refused.
    pub(crate) fn seal(&mut self, frame: &Frame) -> Result<Vec<Vec<u8>>, Error> {
        let bytes = frame.encode();
        if bytes.len() > MAX_FRAME {
            return Err(Error::TooLarge);
        }
        let mut out = Vec::new();
        let mut chunks = bytes.chunks(CHUNK).peekable();
        // An empty frame still takes one message.
        let empty: &[u8] = &[];
        if chunks.peek().is_none() {
            out.push(self.seal_chunk(empty, false));
        }
        while let Some(chunk) = chunks.next() {
            let more = chunks.peek().is_some();
            out.push(self.seal_chunk(chunk, more));
        }
        Ok(out)
    }

    fn seal_chunk(&mut self, chunk: &[u8], more: bool) -> Vec<u8> {
        let mut plain = Vec::with_capacity(chunk.len() + 1);
        plain.push(u8::from(more));
        plain.extend_from_slice(chunk);
        let mut out = vec![0; plain.len() + TAG];
        let n = self
            .noise
            .write_message(&plain, &mut out)
            .expect("a chunk fits a message");
        out.truncate(n);
        out
    }

    /// The frame `msg` completes, if it does.
    pub(crate) fn open(&mut self, msg: &[u8]) -> Result<Option<Frame>, Error> {
        if msg.len() > MAX_MESSAGE {
            return Err(Error::TooLarge);
        }
        let mut plain = vec![0; msg.len()];
        let n = self
            .noise
            .read_message(msg, &mut plain)
            .map_err(|_| Error::Crypto)?;
        let (more, chunk) = match plain[..n].split_first() {
            Some((0, chunk)) => (false, chunk),
            Some((1, chunk)) => (true, chunk),
            _ => return Err(Error::Malformed("chunk")),
        };
        if self.partial.len() + chunk.len() > MAX_FRAME {
            return Err(Error::TooLarge);
        }
        self.partial.extend_from_slice(chunk);
        if more {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.partial);
        Frame::decode(&bytes).map(Some)
    }
}
