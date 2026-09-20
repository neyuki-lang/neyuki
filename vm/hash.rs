//! A small, fast, non-cryptographic hasher for the VM's string-keyed maps.
//!
//! Table fields and globals are looked up on nearly every instruction that
//! touches a name, and the standard SipHash is built to resist collision
//! attacks rather than to be quick on short keys. This is the FxHash scheme
//! used inside rustc: a multiply-and-rotate per word. Keys never come from an
//! untrusted network source in a way that needs DoS resistance.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline(always)]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add_to_hash(u64::from_le_bytes(*chunk));
        }
        if rest.len() >= 4 {
            self.add_to_hash(u32::from_le_bytes(rest[..4].try_into().unwrap()) as u64);
            for &b in &rest[4..] {
                self.add_to_hash(b as u64);
            }
        } else {
            for &b in rest {
                self.add_to_hash(b as u64);
            }
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

/// `FxHashMap::new()` is not available because the hasher is not
/// `RandomState`; this is the equivalent constructor.
pub fn new_map<K, V>() -> FxHashMap<K, V> {
    HashMap::with_hasher(FxBuildHasher::default())
}
