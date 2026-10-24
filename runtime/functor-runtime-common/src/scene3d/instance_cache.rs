//! Retained GPU instance buffers for `Scene.instanced` — the GL-free
//! bookkeeping, generic over the buffer handle so it unit-tests headlessly.
//!
//! The scene stays a plain value; this is shell-side derived state (like
//! physics reconciliation), so it cannot affect determinism, replay, or time
//! travel. A draw whose [`InstanceList`] is the SAME `Arc` as a retained one
//! reuses that buffer with no hashing and no upload (the prelude hands back
//! the same `Arc` for the same Functor Lang list). A different `Arc` falls
//! back to a content hash, so a list rebuilt with identical content still
//! skips the upload; only changed content uploads.

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
    by_hash: HashMap<u64, Retained<B>>,
    // `Arc` address → content hash: the fast path that skips hashing.
    by_ptr: HashMap<usize, u64>,
    // Evicted buffers, reused before creating new ones and released if a
    // whole frame passes without reuse.
    spare: Vec<(B, usize)>,
    frame: u64,
}

impl<B: Copy> Default for InstanceBufferCache<B> {
    fn default() -> Self {
        InstanceBufferCache {
            by_hash: HashMap::new(),
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
        self.by_hash.retain(|_, entry| {
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
        // Same `Arc` as a retained entry: nothing to hash or upload.
        if let Some(hash) = self.by_ptr.get(&ptr).copied() {
            if self
                .by_hash
                .get(&hash)
                .is_some_and(|entry| entry.list.ptr_eq(list))
            {
                let entry = self.by_hash.get_mut(&hash).expect("checked above");
                entry.last_used = frame;
                return Acquired {
                    buffer: entry.buffer,
                    upload: None,
                };
            }
        }
        let bytes = crate::terrain_renderer::slice_bytes(list);
        let hash = content_hash(bytes);
        match self.by_hash.entry(hash) {
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                // Verified byte-for-byte, so a hash collision costs an upload,
                // never a wrong draw.
                let unchanged = crate::terrain_renderer::slice_bytes(&entry.list) == bytes;
                self.by_ptr.remove(&entry.list.identity());
                self.by_ptr.insert(ptr, hash);
                entry.list = list.clone();
                entry.last_used = frame;
                Acquired {
                    buffer: entry.buffer,
                    upload: (!unchanged).then_some(&mut entry.capacity),
                }
            }
            Entry::Vacant(vacant) => {
                let (buffer, capacity) = self.spare.pop().unwrap_or_else(|| (create(), 0));
                self.by_ptr.insert(ptr, hash);
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

/// A fast non-cryptographic hash of the instance bytes — four independent
/// multiply lanes so it runs near memory bandwidth.
fn content_hash(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mix = |h: u64, word: u64| (h ^ word).wrapping_mul(K).rotate_left(31);
    let mut lanes = [K, K.rotate_left(16), K.rotate_left(32), K.rotate_left(48)];
    let mut chunks = bytes.chunks_exact(32);
    for chunk in &mut chunks {
        for (lane, word) in lanes.iter_mut().zip(chunk.chunks_exact(8)) {
            *lane = mix(*lane, u64::from_le_bytes(word.try_into().expect("8 bytes")));
        }
    }
    let mut hash = bytes.len() as u64;
    for word in chunks.remainder().chunks(8) {
        let mut padded = [0u8; 8];
        padded[..word.len()].copy_from_slice(word);
        hash = mix(hash, u64::from_le_bytes(padded));
    }
    lanes.into_iter().fold(hash, mix)
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
    fn draw(cache: &mut InstanceBufferCache<u32>, list: &InstanceList, next: &mut u32) -> (u32, bool) {
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
    fn an_equal_rebuilt_list_skips_the_upload_via_the_hash() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        cache.begin_frame(1);
        assert_eq!(draw(&mut cache, &list(&[1.0, 2.0]), &mut next), (1, true));
        cache.begin_frame(2);
        let rebuilt = list(&[1.0, 2.0]);
        assert_eq!(draw(&mut cache, &rebuilt, &mut next), (1, false));
        // The rebuilt `Arc` is now the retained identity: no rehash needed.
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
        assert_eq!(cache.by_hash.len(), 2);
        assert_eq!(cache.by_ptr.len(), 2);
        // Nothing drawn: the entries evict to spares, then the spares release.
        assert!(cache.begin_frame(20).unwrap().is_empty());
        assert_eq!(cache.begin_frame(21).unwrap().len(), 1);
        assert!(cache.by_hash.is_empty() && cache.by_ptr.is_empty());
        assert_eq!(cache.begin_frame(22).unwrap().len(), 1);
    }

    #[test]
    fn a_repeated_frame_id_does_not_evict() {
        let mut cache = InstanceBufferCache::<u32>::default();
        cache.begin_frame(5);
        assert!(cache.begin_frame(5).is_none());
    }

    #[test]
    fn hash_collisions_upload_instead_of_drawing_stale_data() {
        let mut cache = InstanceBufferCache::default();
        let mut next = 0;
        let a = list(&[1.0]);
        let b = list(&[2.0]);
        cache.begin_frame(1);
        draw(&mut cache, &a, &mut next);
        // Force B onto A's hash slot to simulate a collision.
        let entry = cache.by_hash.remove(&content_hash(crate::terrain_renderer::slice_bytes(&a)));
        cache
            .by_hash
            .insert(content_hash(crate::terrain_renderer::slice_bytes(&b)), entry.unwrap());
        cache.by_ptr.clear();
        assert_eq!(draw(&mut cache, &b, &mut next), (1, true));
    }
}
