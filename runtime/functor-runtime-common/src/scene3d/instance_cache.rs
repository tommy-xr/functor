//! Retained GPU instance buffers for `Scene.instanced` — the GL-free
//! bookkeeping, generic over the buffer handle so it unit-tests headlessly.
//!
//! The scene stays a plain value; this is shell-side derived state (like
//! physics reconciliation), so it cannot affect determinism, replay, or time
//! travel. A draw whose [`InstanceList`] is the SAME `Arc` as a retained one
//! reuses that buffer with no hashing and no upload (the prelude hands back
//! the same `Arc` for the same Functor Lang list). A different `Arc` falls
//! back to a content key verified byte-for-byte, so a list rebuilt with
//! identical content still skips the upload; only changed content uploads.
//!
//! The key hashes a fixed SAMPLE of the records, not all of them: hashing a
//! 28k-instance list (1.4 MB) costs about 3× a memcpy of it — comparable to
//! the upload it would avoid — while the sample is constant-time, and the
//! verifying compare stops at the first differing byte (immediately, for a
//! list that moves every frame). Two lists drawn in the SAME frame that
//! differ only outside the sample share a key and re-upload each other's
//! buffer every draw — correct, just unretained, as before this cache.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use super::InstanceList;

/// An entry not drawn in the previous frame is evicted at the next frame's
/// start (the terrain renderer's retention rule), so a game that rebuilds
/// changing lists holds at most two buffers per node.
const RETAIN_FRAMES: u64 = 1;

struct Retained<B> {
    // Pins the `Arc`: its address cannot be reused while retained, so a
    // pointer match below is a sound identity check.
    list: InstanceList,
    buffer: B,
    capacity: usize,
    last_used: u64,
}

/// One draw's buffer, and whether the caller must upload into it.
pub(crate) struct Acquired<'a, B> {
    pub buffer: B,
    /// `Some(capacity)`: upload the list into `buffer` (growing the
    /// high-water `capacity` as needed). `None`: it already holds the list.
    pub upload: Option<&'a mut usize>,
}

pub(crate) struct InstanceBufferCache<B> {
    by_key: HashMap<u64, Retained<B>>,
    // `Arc` address → content key: the fast path that skips keying.
    by_ptr: HashMap<usize, u64>,
    // Evicted buffers, reused before creating new ones and released if a
    // whole frame passes without reuse.
    spare: Vec<(B, usize)>,
    frame: u64,
}

impl<B: Copy> Default for InstanceBufferCache<B> {
    fn default() -> Self {
        InstanceBufferCache {
            by_key: HashMap::new(),
            by_ptr: HashMap::new(),
            spare: Vec::new(),
            frame: 0,
        }
    }
}

impl<B: Copy> InstanceBufferCache<B> {
    /// Start `frame`: evict entries not drawn recently into the spare pool
    /// and return the buffers to delete. `None` when `frame` already began
    /// (a stereo pair shares one frame id).
    pub fn begin_frame(&mut self, frame: u64) -> Option<Vec<B>> {
        if frame == self.frame {
            return None;
        }
        self.frame = frame;
        let released = self.spare.drain(..).map(|(buffer, _)| buffer).collect();
        let (by_ptr, spare) = (&mut self.by_ptr, &mut self.spare);
        self.by_key.retain(|_, entry| {
            let recent = frame.wrapping_sub(entry.last_used) <= RETAIN_FRAMES;
            if !recent {
                by_ptr.remove(&entry.list.identity());
                spare.push((entry.buffer, entry.capacity));
            }
            recent
        });
        Some(released)
    }

    /// The buffer holding `list` for this draw; `create` makes a new one when
    /// no spare is free.
    pub fn acquire(&mut self, list: &InstanceList, create: impl FnOnce() -> B) -> Acquired<'_, B> {
        let frame = self.frame;
        let ptr = list.identity();
        // Same `Arc` as a retained entry: nothing to compare or upload.
        if let Some(key) = self.by_ptr.get(&ptr).copied() {
            if self
                .by_key
                .get(&key)
                .is_some_and(|entry| entry.list.ptr_eq(list))
            {
                let entry = self.by_key.get_mut(&key).expect("checked above");
                entry.last_used = frame;
                return Acquired {
                    buffer: entry.buffer,
                    upload: None,
                };
            }
        }
        let key = content_key(list);
        match self.by_key.entry(key) {
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                // Verified byte-for-byte, so a key collision costs an upload,
                // never a wrong draw.
                let slice_bytes = crate::terrain_renderer::slice_bytes::<super::InstanceData>;
                let unchanged = slice_bytes(&entry.list) == slice_bytes(list);
                self.by_ptr.remove(&entry.list.identity());
                self.by_ptr.insert(ptr, key);
                entry.list = list.clone();
                entry.last_used = frame;
                Acquired {
                    buffer: entry.buffer,
                    upload: (!unchanged).then_some(&mut entry.capacity),
                }
            }
            Entry::Vacant(vacant) => {
                let (buffer, capacity) = self.spare.pop().unwrap_or_else(|| (create(), 0));
                self.by_ptr.insert(ptr, key);
                let entry = vacant.insert(Retained {
                    list: list.clone(),
                    buffer,
                    capacity,
                    last_used: frame,
                });
                Acquired {
                    buffer: entry.buffer,
                    upload: Some(&mut entry.capacity),
                }
            }
        }
    }
}

/// Records hashed into the content key, evenly spaced (plus the last).
const KEY_SAMPLES: usize = 64;

/// A constant-time key over the list's length and a fixed sample of its
/// records (see the module doc for why not all of them).
fn content_key(list: &[super::InstanceData]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = crate::host_registry::FxHasher::default();
    hasher.write_usize(list.len());
    let stride = list.len().div_ceil(KEY_SAMPLES).max(1);
    for record in list.iter().step_by(stride).chain(list.last()) {
        hasher.write(crate::terrain_renderer::slice_bytes(std::slice::from_ref(record)));
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::InstanceData;

    fn list(xs: &[f32]) -> InstanceList {
        xs.iter()
            .map(|x| InstanceData::at([*x, 0.0, 0.0]))
            .collect::<Vec<_>>()
            .into()
    }

    /// Acquire `list`, returning (buffer, uploaded?) and counting creations.
    fn draw(
        cache: &mut InstanceBufferCache<u32>,
        list: &InstanceList,
        next: &mut u32,
    ) -> (u32, bool) {
        let acquired = cache.acquire(list, || {
            *next += 1;
            *next
        });
        let uploaded = match acquired.upload {
            Some(capacity) => {
                *capacity = (*capacity).max(list.len());
                true
            }
            None => false,
        };
        (acquired.buffer, uploaded)
    }

    #[test]
    fn the_same_list_uploads_once() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        let trees = list(&[1.0, 2.0, 3.0]);
        cache.begin_frame(1);
        assert_eq!(draw(&mut cache, &trees, &mut next), (1, true));
        // The depth and forward passes of one frame share the upload…
        assert_eq!(draw(&mut cache, &trees, &mut next), (1, false));
        // …and so do later frames.
        for frame in 2..10 {
            cache.begin_frame(frame);
            assert_eq!(draw(&mut cache, &trees, &mut next), (1, false));
        }
        assert_eq!(next, 1);
    }

    #[test]
    fn an_equal_rebuilt_list_skips_the_upload_via_the_content_key() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        cache.begin_frame(1);
        assert_eq!(draw(&mut cache, &list(&[1.0, 2.0]), &mut next), (1, true));
        cache.begin_frame(2);
        let rebuilt = list(&[1.0, 2.0]);
        assert_eq!(draw(&mut cache, &rebuilt, &mut next), (1, false));
        // The rebuilt `Arc` is now the retained identity: no re-keying.
        assert_eq!(draw(&mut cache, &rebuilt, &mut next), (1, false));
    }

    #[test]
    fn changed_content_uploads() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        cache.begin_frame(1);
        assert_eq!(draw(&mut cache, &list(&[1.0, 2.0]), &mut next), (1, true));
        cache.begin_frame(2);
        let (_, uploaded) = draw(&mut cache, &list(&[1.0, 2.5]), &mut next);
        assert!(uploaded);
    }

    #[test]
    fn stale_entries_are_evicted_and_their_buffers_recycled() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        // A list that changes every frame: frame 1 and 2 need two buffers;
        // from then on the evicted one is recycled — no creation churn.
        for frame in 1..20u64 {
            let released = cache.begin_frame(frame).expect("new frame");
            assert!(released.is_empty(), "spares are reused, not released");
            let (_, uploaded) = draw(&mut cache, &list(&[frame as f32]), &mut next);
            assert!(uploaded);
        }
        assert_eq!(next, 2);
        assert_eq!(cache.by_key.len(), 2);
        assert_eq!(cache.by_ptr.len(), 2);
        // Nothing drawn: the entries evict to spares, then the spares release.
        assert!(cache.begin_frame(20).unwrap().is_empty());
        assert_eq!(cache.begin_frame(21).unwrap().len(), 1);
        assert!(cache.by_key.is_empty() && cache.by_ptr.is_empty());
        assert_eq!(cache.begin_frame(22).unwrap().len(), 1);
    }

    #[test]
    fn a_repeated_frame_id_does_not_evict() {
        let mut cache = InstanceBufferCache::<u32>::default();
        cache.begin_frame(5);
        assert!(cache.begin_frame(5).is_none());
    }

    #[test]
    fn a_change_outside_the_key_sample_still_uploads() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        let xs: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let a = list(&xs);
        let mut changed = xs.clone();
        changed[1] = -1.0; // record 1 is not sampled (stride 16)
        let b = list(&changed);
        assert_eq!(content_key(&a), content_key(&b));
        cache.begin_frame(1);
        assert_eq!(draw(&mut cache, &a, &mut next), (1, true));
        cache.begin_frame(2);
        assert_eq!(draw(&mut cache, &b, &mut next), (1, true));
    }
}
