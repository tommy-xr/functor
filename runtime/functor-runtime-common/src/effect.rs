use fable_library_rust::Native_::Func1;

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

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
        }
    }
}
