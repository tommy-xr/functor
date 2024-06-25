use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

pub struct AssetHandle<T> {
    state: RefCell<Option<AssetState<T>>>,
    fallback_asset: Arc<T>,
}

impl<T> AssetHandle<T> {
    pub fn get(&self) -> Arc<T> {
        let mut state = self.state.borrow_mut();
        if let Some(asset_state) = state.take() {
            let new_state = asset_state.ensure_loaded();
            let ret = match &new_state {
                AssetState::Loaded(asset) => asset.clone(),
                AssetState::Loading(_) => self.fallback_asset.clone(),
            };
            *state = Some(new_state);
            ret
        } else {
            panic!("Should never happen")
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
}

pub enum AssetState<T> {
    Loading(Pin<Box<dyn Future<Output = Result<Arc<T>, String>>>>),
    Loaded(Arc<T>),
}

impl<T> AssetState<T> {
    pub fn ensure_loaded(self) -> Self {
        match self {
            AssetState::Loaded(_) => self,
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
                    // TODO: More robust error handling...
                    panic!("Failed to load asset: {}", e);
                }
                Poll::Pending => {
                    println!("Waiting for texture to load...");
                    AssetState::Loading(future)
                }
            }
        } else {
            self
        }
    }
}
