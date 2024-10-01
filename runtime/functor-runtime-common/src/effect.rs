#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
}

impl<T: Clone + 'static> Effect<T> {
    pub fn none() -> Effect<T> {
        Effect::None
    }

    pub fn wrapped(data: T) -> Effect<T> {
        Effect::Wrapped(data)
    }
}
