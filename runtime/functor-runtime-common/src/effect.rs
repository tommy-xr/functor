use fable_library_rust::Native_::Func1;
use std::cell::UnsafeCell;
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

struct UnsafeFuture<F>(UnsafeCell<F>);

impl<F> UnsafeFuture<F> {
    fn new(future: F) -> Self {
        UnsafeFuture(UnsafeCell::new(future))
    }
}

unsafe impl<F: Future + Send> Send for UnsafeFuture<F> {}
unsafe impl<F: Future + Sync> Sync for UnsafeFuture<F> {}

impl<F: Future> EffectFuture<F::Output> for UnsafeFuture<F> {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: We know that no other references to the future exist
        // because `poll` takes `&self`, not `&mut self`.
        unsafe {
            let future = &mut *self.0.get();
            Pin::new_unchecked(future).poll(cx)
        }
    }
}

impl<T: Clone + 'static> Effect<T> {
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
        Effect::Pending(Rc::new(UnsafeFuture::new(future)))
    }

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
            Effect::Pending(fut) => {
                let mapped_fut = Rc::new(UnsafeFuture::new(MappedFuture {
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
