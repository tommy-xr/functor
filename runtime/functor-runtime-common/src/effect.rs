use fable_library_rust::{NativeArray_, Native_::Func1};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
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

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
        }
    }

    pub fn run(effect: Effect<T>) -> NativeArray_::Array<T> {
        match effect {
            Effect::None => NativeArray_::array_from(vec![]),
            Effect::Wrapped(v) => NativeArray_::array_from(vec![v]),
        }
    }
}
