use std::cell::{Ref, RefCell};

pub trait RenderableAsset {
    type HydratedType;
    type OptionsType;

    fn hydrate(
        &self,
        gl_context: &glow::Context,
        options: &Self::OptionsType,
    ) -> Self::HydratedType;
}

enum RenderableAssetState<T: RenderableAsset> {
    Dehydrated(T, T::OptionsType),
    Hydrated(T::HydratedType),
}

pub struct RuntimeRenderableAsset<T: RenderableAsset> {
    state: RefCell<Option<RenderableAssetState<T>>>,
}

impl<T: RenderableAsset> RuntimeRenderableAsset<T> {
    pub fn new(asset: T, options: T::OptionsType) -> Self {
        RuntimeRenderableAsset {
            state: RefCell::new(Some(RenderableAssetState::Dehydrated(asset, options))),
        }
    }

    pub fn ensure_hydrated(&self, gl_context: &glow::Context) -> &Self {
        let mut state = self.state.borrow_mut();
        if let Some(asset_state) = state.take() {
            let new_state = match asset_state {
                RenderableAssetState::Hydrated(_) => asset_state,
                RenderableAssetState::Dehydrated(asset, options) => {
                    println!("Hydrating asset!");
                    RenderableAssetState::Hydrated(asset.hydrate(gl_context, &options))
                }
            };
            *state = Some(new_state);
        }
        self
    }

    pub fn get_opt(&self) -> Option<Ref<T::HydratedType>> {
        if let Some(RenderableAssetState::Hydrated(ref loaded)) = *self.state.borrow() {
            Some(Ref::map(self.state.borrow(), |s| {
                if let Some(RenderableAssetState::Hydrated(ref l)) = *s {
                    l
                } else {
                    unreachable!()
                }
            }))
        } else {
            None
        }
    }

    pub fn get(&self, context: &glow::Context) -> Ref<T::HydratedType> {
        self.ensure_hydrated(context);
        self.get_opt().unwrap()
    }
}
