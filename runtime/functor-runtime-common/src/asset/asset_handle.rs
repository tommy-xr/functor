use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub struct AssetHandle<T> {
    state: RefCell<Option<AssetState<T>>>,
    fallback_asset: Arc<T>,
}

impl<T> AssetHandle<T> {
    pub fn get(&self) -> Arc<T> {
        match self.poll_state() {
            AssetPollState::Loaded(asset) => asset,
            AssetPollState::Loading | AssetPollState::Failed => self.fallback_asset.clone(),
        }
    }

    pub fn new<F>(future: F, fallback_asset: Arc<T>) -> AssetHandle<T>
    where
        F: Future<Output = Result<Arc<T>, String>> + 'static,
    {
        AssetHandle {
            state: RefCell::new(Some(AssetState::Loading(Box::pin(future)))),
            fallback_asset,
        }
    }

    /// Advance a pending load and report the true state — unlike `get`, never
    /// substitutes the fallback. For assets assembled from several files (a
    /// cubemap's six faces) that must ALL be ready before GPU hydration.
    pub fn poll_state(&self) -> AssetPollState<T> {
        let mut state = self.state.borrow_mut();
        if let Some(asset_state) = state.take() {
            let new_state = asset_state.ensure_loaded();
            let ret = match &new_state {
                AssetState::Loaded(asset) => AssetPollState::Loaded(asset.clone()),
                AssetState::Loading(_) => AssetPollState::Loading,
                AssetState::Failed => AssetPollState::Failed,
            };
            *state = Some(new_state);
            ret
        } else {
            panic!("Should never happen")
        }
    }
}

/// The observable state of an [`AssetHandle`], from [`AssetHandle::poll_state`].
pub enum AssetPollState<T> {
    Loading,
    Loaded(Arc<T>),
    Failed,
}

pub enum AssetState<T> {
    Loading(Pin<Box<dyn Future<Output = Result<Arc<T>, String>>>>),
    Loaded(Arc<T>),
    // Loading failed (e.g. the file is missing); the handle keeps serving
    // the fallback asset instead of crashing the runtime.
    Failed,
}

impl<T> AssetState<T> {
    pub fn ensure_loaded(self) -> Self {
        match self {
            AssetState::Loaded(_) | AssetState::Failed => self,
            AssetState::Loading(..) => self.poll_load(),
        }
    }

    pub fn poll_load(self) -> Self {
        if let Self::Loading(mut future) = self {
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            match Future::poll(Pin::new(&mut future), &mut cx) {
                Poll::Ready(Ok(texture_data)) => AssetState::Loaded(texture_data),
                Poll::Ready(Err(e)) => {
                    crate::events::emit(crate::events::RuntimeEvent::AssetError {
                        path: None,
                        message: e,
                    });
                    AssetState::Failed
                }
                Poll::Pending => AssetState::Loading(future),
            }
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_state_reports_loaded_without_fallback() {
        let handle = AssetHandle::new(async { Ok(Arc::new(7)) }, Arc::new(0));
        match handle.poll_state() {
            AssetPollState::Loaded(v) => assert_eq!(*v, 7),
            _ => panic!("expected Loaded"),
        }
        // get() agrees once loaded.
        assert_eq!(*handle.get(), 7);
    }

    #[test]
    fn poll_state_reports_failed_where_get_substitutes_the_fallback() {
        let handle = AssetHandle::new(async { Err("nope".to_string()) }, Arc::new(42));
        assert!(matches!(handle.poll_state(), AssetPollState::Failed));
        // get() keeps its fallback contract.
        assert_eq!(*handle.get(), 42);
    }
}
