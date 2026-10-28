//! Retained GPU instance buffers for `Scene.instanced` — the GL-free
//! bookkeeping, generic over the buffer handle so it unit-tests headlessly.
//!
//! The scene stays a plain value; this is shell-side derived state (like
//! physics reconciliation), so it cannot affect determinism, replay, or time
//! travel. Entries are found by a constant-time content key. A draw whose
//! [`InstanceList`] is the SAME `Arc` as the retained one reuses its buffer
//! with no compare and no upload (the prelude hands back the same `Arc` for
//! the same Functor Lang list); any other list is verified byte-for-byte, so
//! a list rebuilt with identical content still skips the upload and only
//! changed content uploads.
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

use super::{Aabb, InstanceList};

/// An entry not drawn in the previous frame is evicted at the next frame's
/// start (the terrain renderer's retention rule), so a game that rebuilds
/// changing lists holds at most two buffers per node.
const RETAIN_FRAMES: u64 = 1;

struct Retained<B> {
    // Pins the `Arc`: its address cannot be reused while retained, so a
    // pointer match is a sound identity check.
    list: InstanceList,
    buffer: B,
    capacity: usize,
    extent: InstanceExtent,
    last_used: u64,
}

/// Where a list's copies reach, for culling: the box of their positions,
/// and the most any copy stretches its template (scale times the growth a
/// non-unit quaternion adds in the instance shader's `rotate`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct InstanceExtent {
    pub positions: Aabb,
    pub stretch: f32,
}

impl InstanceExtent {
    fn of(list: &[super::InstanceData]) -> InstanceExtent {
        let mut positions = Aabb::empty();
        let mut stretch = 0.0f32;
        for copy in list {
            positions.extend(copy.position.into());
            // `rotate(q, v)` = (1 - |q|²) v + |q|² R v, so |q|² > 1 grows v
            // by up to 2|q|² - 1.
            let q2: f32 = copy.rotation.iter().map(|c| c * c).sum();
            let scale = copy.scale.iter().fold(0.0f32, |s, a| s.max(a.abs()));
            stretch = stretch.max(scale * (2.0 * q2 - 1.0).max(1.0));
        }
        InstanceExtent { positions, stretch }
    }
}

/// One draw's buffer, and whether the caller must upload into it.
pub(crate) struct Acquired<'a, B> {
    pub buffer: B,
    /// `Some(capacity)`: upload the list into `buffer` (growing the
    /// high-water `capacity` as needed). `None`: it already holds the list.
    pub upload: Option<&'a mut usize>,
    pub extent: InstanceExtent,
}

pub(crate) struct InstanceBufferCache<B> {
    entries: HashMap<u64, Retained<B>>,
    // Evicted buffers, reused before creating new ones and released if a
    // whole frame passes without reuse.
    spare: Vec<(B, usize)>,
    frame: u64,
}

impl<B: Copy> Default for InstanceBufferCache<B> {
    fn default() -> Self {
        InstanceBufferCache {
            entries: HashMap::new(),
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
        let spare = &mut self.spare;
        self.entries.retain(|_, entry| {
            let recent = frame.wrapping_sub(entry.last_used) <= RETAIN_FRAMES;
            if !recent {
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
        match self.entries.entry(content_key(list)) {
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                // The same `Arc` needs no compare; any other list is verified
                // byte-for-byte, so a key collision costs an upload, never a
                // wrong draw.
                let slice_bytes = crate::terrain_renderer::slice_bytes::<super::InstanceData>;
                let unchanged =
                    entry.list.ptr_eq(list) || slice_bytes(&entry.list) == slice_bytes(list);
                entry.list = list.clone();
                entry.last_used = frame;
                if !unchanged {
                    entry.extent = InstanceExtent::of(list);
                }
                Acquired {
                    buffer: entry.buffer,
                    upload: (!unchanged).then_some(&mut entry.capacity),
                    extent: entry.extent,
                }
            }
            Entry::Vacant(vacant) => {
                let (buffer, capacity) = self.spare.pop().unwrap_or_else(|| (create(), 0));
                let entry = vacant.insert(Retained {
                    list: list.clone(),
                    buffer,
                    capacity,
                    extent: InstanceExtent::of(list),
                    last_used: frame,
                });
                Acquired {
                    buffer: entry.buffer,
                    upload: Some(&mut entry.capacity),
                    extent: entry.extent,
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
        // The rebuilt `Arc` is now the retained one: no byte compare.
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
        assert_eq!(cache.entries.len(), 2);
        // Nothing drawn: the entries evict to spares, then the spares release.
        assert!(cache.begin_frame(20).unwrap().is_empty());
        assert_eq!(cache.begin_frame(21).unwrap().len(), 1);
        assert!(cache.entries.is_empty());
        assert_eq!(cache.begin_frame(22).unwrap().len(), 1);
    }

    #[test]
    fn the_extent_covers_scale_and_a_non_unit_rotation() {
        let mut far = InstanceData::at([5.0, 0.0, -1.0]);
        far.scale = [1.0, -3.0, 1.0];
        let mut stretched = InstanceData::at([0.0; 3]);
        // |q|² = 4: the shader's `rotate` grows vectors up to 7×.
        stretched.rotation = [2.0, 0.0, 0.0, 0.0];
        let extent = InstanceExtent::of(&[far, InstanceData::at([0.0; 3])]);
        assert_eq!(extent.positions, crate::scene3d::Aabb::new([0.0, 0.0, -1.0], [5.0, 0.0, 0.0]));
        assert_eq!(extent.stretch, 3.0);
        assert_eq!(InstanceExtent::of(&[stretched]).stretch, 7.0);
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
