use std::fmt;

use fable_library_rust::{NativeArray_, Native_::Func1};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
    Pending(Rc<dyn EffectFuture<T>>),
}

pub trait EffectFuture<T> {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<T>;
}

/// A shared, fused future backing a `Pending` effect.
///
/// `Effect` derives `Clone` and `Pending` holds the future behind an `Rc`, so
/// multiple effect clones can poll the *same* future. To keep that sound we:
///   1. resolve the inner future at most once, then cache its output, and
///   2. serve every later poll (from any clone) the cached value rather than
///      re-polling a completed future (which most futures panic on).
/// `RefCell` enforces a single `&mut` to the future at a time; the runtime only
/// ever polls single-threaded, so this type is intentionally `!Send`/`!Sync`.
struct FusedFuture<F: Future> {
    state: RefCell<FusedState<F>>,
}

enum FusedState<F: Future> {
    Pending(F),
    Ready(F::Output),
}

impl<F: Future> FusedFuture<F> {
    fn new(future: F) -> Self {
        FusedFuture {
            state: RefCell::new(FusedState::Pending(future)),
        }
    }
}

impl<F: Future> EffectFuture<F::Output> for FusedFuture<F>
where
    F::Output: Clone,
{
    fn poll(&self, cx: &mut Context<'_>) -> Poll<F::Output> {
        let mut state = self.state.borrow_mut();
        let polled = match &mut *state {
            // Already resolved: hand back the cached output, never re-poll.
            FusedState::Ready(value) => return Poll::Ready(value.clone()),
            FusedState::Pending(future) => {
                // SAFETY: the future is owned by this `RefCell` inside an `Rc`
                // and is never moved while pending; `RefCell` guarantees a single
                // `&mut` at a time and polling is single-threaded.
                let pinned = unsafe { Pin::new_unchecked(future) };
                pinned.poll(cx)
            }
        };
        if let Poll::Ready(value) = &polled {
            *state = FusedState::Ready(value.clone());
        }
        polled
    }
}

// Implement Debug manually so it doesn't require `T: Debug`. This lets types
// that embed an Effect (e.g. the generated Game record) derive Debug, while
// keeping effects opaque - the payload is plain data but not necessarily printable.
impl<T: Clone + 'static> fmt::Debug for Effect<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::None => f.write_str("Effect::None"),
            Effect::Wrapped(_) => f.write_str("Effect::Wrapped(..)"),
        }
    }
}

impl<T: Clone + 'static> Effect<T> {
    pub fn is_none(effect: &Effect<T>) -> bool {
        match effect {
            Effect::None => true,
            _ => false,
        }
    }

    pub fn none() -> Effect<T> {
        Effect::None
    }

    pub fn wrapped(data: T) -> Effect<T> {
        Effect::Wrapped(data)
    }

    pub fn pending<F>(future: F) -> Effect<T>
    where
        F: Future<Output = T> + 'static,
    {
        Effect::Pending(Rc::new(FusedFuture::new(future)))
    }

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
            Effect::Pending(fut) => {
                let mapped_fut = Rc::new(FusedFuture::new(MappedFuture {
                    inner: fut,
                    mapping: Rc::new(mapping),
                }));
                Effect::Pending(mapped_fut)
            }
        }
    }

    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        match self {
            Effect::None => Poll::Ready(None),
            Effect::Wrapped(data) => Poll::Ready(Some(data.clone())),
            Effect::Pending(fut) => {
                match fut.poll(cx) {
                    Poll::Ready(data) => {
                        // Replace the Pending variant with Wrapped
                        *self = Effect::Wrapped(data.clone());
                        Poll::Ready(Some(data))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }

    pub fn run(effect: Effect<T>) -> NativeArray_::Array<T> {
        match effect {
            Effect::None => NativeArray_::array_from(vec![]),
            Effect::Wrapped(v) => NativeArray_::array_from(vec![v]),
            // TODO(#61): Pending effects are silently dropped here. `run` is what
            // the runtime drain loop (Runtime.fs `GameExecutor.tick`) calls, and
            // it discards the dequeued effect — so a `Pending` yields no message
            // and is never polled to completion or re-enqueued. Wiring async
            // effects into the loop (poll + re-enqueue while pending, à la
            // asset_handle.rs's noop_waker) is tracked by the cmd/data-model
            // refactor in issue #61.
            Effect::Pending(_) => NativeArray_::array_from(vec![]),
        }
    }
}

struct MappedFuture<T, U> {
    inner: Rc<dyn EffectFuture<T>>,
    mapping: Rc<Func1<T, U>>,
}

impl<T: Clone + 'static, U: Clone + 'static> Future for MappedFuture<T, U> {
    type Output = U;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.poll(cx) {
            Poll::Ready(value) => Poll::Ready((self.mapping)(value)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_none_distinguishes_variants() {
        assert!(Effect::is_none(&Effect::<i32>::none()));
        assert!(!Effect::is_none(&Effect::wrapped(1)));
    }

    #[test]
    fn run_none_yields_no_messages() {
        let msgs = Effect::<i32>::run(Effect::none());
        assert_eq!(NativeArray_::count(msgs), 0);
    }

    #[test]
    fn run_wrapped_yields_single_message() {
        let msgs = Effect::run(Effect::wrapped(42));
        assert_eq!(NativeArray_::count(msgs.clone()), 1);
        assert!(NativeArray_::contains(msgs, 42));
    }

    #[test]
    fn pending_resolves_once_and_caches_across_clones() {
        use std::future::ready;
        use std::task::{Context, Poll};

        let eff = Effect::pending(ready(7));
        let mut first = eff.clone();
        let mut second = eff;

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        // One clone drives the shared future to completion.
        assert_eq!(first.poll(&mut cx), Poll::Ready(Some(7)));
        // The other clone must read the cached result rather than re-poll the
        // already-completed future (std::future::Ready panics if polled twice).
        assert_eq!(second.poll(&mut cx), Poll::Ready(Some(7)));
    }
}
