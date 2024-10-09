use std::{cell::RefCell, collections::VecDeque, fmt, sync::Arc};

use crate::Effect;

#[derive(Clone)]
pub struct EffectQueue<T: Clone + 'static> {
    queue: Arc<RefCell<VecDeque<Effect<T>>>>,
}

impl<T: Clone + 'static> EffectQueue<T> {
    pub fn new() -> EffectQueue<T> {
        EffectQueue {
            queue: Arc::new(RefCell::new(VecDeque::new())),
        }
    }

    pub fn count(effect_queue: &EffectQueue<T>) -> i32 {
        effect_queue.queue.borrow().len() as i32
    }

    pub fn enqueue(effect_queue: &EffectQueue<T>, effect: Effect<T>) {
        if !Effect::is_none(&effect) {
            effect_queue.queue.borrow_mut().push_front(effect);
        }
    }

    pub fn dequeue(effect_queue: &EffectQueue<T>) -> Option<Effect<T>> {
        effect_queue.queue.borrow_mut().pop_back()
    }
}

// Implement Debug manually
impl<T: Clone + 'static> fmt::Debug for EffectQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Build the debug representation
        f.debug_struct("EffectQueue").finish()
    }
}
