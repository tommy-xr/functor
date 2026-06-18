//! Networking core (Phase 0 of `docs/multiplayer.md`).
//!
//! This module defines the transport-agnostic types shared by every protocol
//! (HTTP, WebSocket, TCP/UDP, WebRTC) and a deterministic in-memory network
//! (`VirtualNet`) used to drive multiplayer games headlessly in tests.
//!
//! Nothing here opens a real socket. The real, async transports (tokio on
//! native, web-sys on wasm) land in later phases behind the same `NetEvent` /
//! `ConnectionId` vocabulary, so game code and the connection manager are
//! identical whether they run over a real network or the virtual one.

mod connection;
mod inbox;
mod registry;
mod virtual_net;

pub use connection::*;
pub use inbox::*;
pub use registry::*;
pub use virtual_net::*;

/// Stable identifier for one connection, assigned by the runtime and reported to
/// the game via [`NetEvent::Connected`]. It is plain data: the game stores it in
/// its model and names it when sending (so outbound `Effect`s stay plain data and
/// survive hot reload — see `docs/multiplayer.md`).
pub type ConnectionId = u64;

/// Inbound events surfaced to the game through a networking `Sub`. The decoder a
/// `Sub` carries maps these to the game's own `'msg`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    /// A connection became usable. For a client this is *our* connection to the
    /// server; for a server-side `listen` this is a newly-accepted client.
    Connected(ConnectionId),
    /// Bytes arrived on a connection.
    Message(ConnectionId, Vec<u8>),
    /// A connection closed (peer left, transport dropped it, or we closed it).
    Disconnected(ConnectionId),
    /// A transport-level error on a connection. The connection is considered dead.
    Error(ConnectionId, String),
}

/// Shape of one network link, in *logical ticks* rather than wall-clock so the
/// virtual network is fully deterministic. The harness advances ticks; real
/// transports will translate ticks ⇄ time at the edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkProfile {
    /// Base one-way delay, in ticks, before a sent packet can be delivered.
    pub latency_ticks: u32,
    /// Maximum extra delay added on top of `latency_ticks`, drawn deterministically
    /// from the network's seeded RNG in `[0, jitter_ticks]`.
    pub jitter_ticks: u32,
    /// Probability in `[0.0, 1.0]` that a given packet is dropped.
    pub loss: f32,
    /// When `false`, delivery order is forced to match send order on each
    /// direction even if jitter would otherwise reorder packets.
    pub reorder: bool,
}

impl LinkProfile {
    /// A clean link: deliver next tick, no jitter, no loss, in order.
    pub const PERFECT: LinkProfile = LinkProfile {
        latency_ticks: 1,
        jitter_ticks: 0,
        loss: 0.0,
        reorder: false,
    };
}

impl Default for LinkProfile {
    fn default() -> Self {
        LinkProfile::PERFECT
    }
}

/// Deterministic, allocation-free PRNG (SplitMix64). The virtual network must be
/// reproducible from a seed, so it never touches `rand`/wall-clock; this provides
/// the jitter and loss draws.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A float in `[0.0, 1.0)`.
    pub fn next_f32(&mut self) -> f32 {
        // Top 24 bits give a uniformly-spaced float in [0, 1).
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// An integer in `[lo, hi]` inclusive.
    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            lo
        } else {
            lo + (self.next_u64() % (hi - lo + 1) as u64) as u32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_for_a_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn rng_floats_are_in_unit_interval() {
        let mut r = Rng::new(7);
        for _ in 0..1000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f), "{f} out of range");
        }
    }

    #[test]
    fn rng_range_is_inclusive_and_bounded() {
        let mut r = Rng::new(123);
        let mut saw_lo = false;
        let mut saw_hi = false;
        for _ in 0..1000 {
            let v = r.range_u32(2, 5);
            assert!((2..=5).contains(&v));
            saw_lo |= v == 2;
            saw_hi |= v == 5;
        }
        assert!(saw_lo && saw_hi, "range should reach both endpoints");
    }
}
