use std::fmt;

use fable_library_rust::{NativeArray_, Native_::Func1};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
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
}
