/// The producer seam this runner's loops consume. The trait itself lives in
/// `functor_runtime_common::protocol` (it is the producer side of the
/// logic↔runtime protocol, shared with the web runtime); `Game` is this
/// crate's historical name for it. Impls here: `StaticGame`, `HotReloadGame`,
/// and the throwaway `MleGame` spike.
pub use functor_runtime_common::protocol::GameProducer as Game;
