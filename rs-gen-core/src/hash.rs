use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplier of the hasher, from the multiply-rotate hasher used inside rustc (FxHash).
const MULTIPLIER: u64 = 0x517c_c1b7_2722_0a95;

/// Fast non-cryptographic hasher for the internal tables.
///
/// Several times faster than the default SipHash on short keys such as n-gram prefixes.
/// It does not resist keys crafted to collide (HashDoS): the tables it serves are filled
/// from the training files only, generation requests just read them.
#[derive(Default, Clone, Copy)]
pub(crate) struct FastHasher {
	/// Current hash state.
	hash: u64,
}

impl FastHasher {
	/// Mixes one 64-bit word into the hash.
	fn add(&mut self, word: u64) {
		self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(MULTIPLIER);
	}
}

impl Hasher for FastHasher {
	fn write(&mut self, bytes: &[u8]) {
		let (words, rest) = bytes.as_chunks::<8>();
		for word in words {
			self.add(u64::from_le_bytes(*word));
		}
		if !rest.is_empty() {
			let mut word = [0u8; 8];
			word[..rest.len()].copy_from_slice(rest);
			self.add(u64::from_le_bytes(word));
		}
	}

	fn write_u8(&mut self, value: u8) {
		self.add(u64::from(value));
	}

	fn write_u32(&mut self, value: u32) {
		self.add(u64::from(value));
	}

	fn write_u64(&mut self, value: u64) {
		self.add(value);
	}

	fn write_usize(&mut self, value: usize) {
		self.add(value as u64);
	}

	fn finish(&self) -> u64 {
		// The last multiplication leaves its best bits at the top: bring them to the bottom,
		// where the table picks its bucket
		self.hash.rotate_left(26)
	}
}

/// Builds `FastHasher`s for a table.
pub(crate) type FastBuildHasher = BuildHasherDefault<FastHasher>;

/// Hash map using `FastHasher`.
pub(crate) type FastHashMap<K, V> = HashMap<K, V, FastBuildHasher>;

/// Hash set using `FastHasher`.
pub(crate) type FastHashSet<T> = HashSet<T, FastBuildHasher>;

#[cfg(test)]
mod tests {
	use super::*;
	use std::hash::BuildHasher;

	fn hash_of(value: &str) -> u64 {
		FastBuildHasher::default().hash_one(value)
	}

	#[test]
	fn equal_keys_have_equal_hashes() {
		assert_eq!(hash_of("<été"), hash_of("<été"));
	}

	#[test]
	fn close_keys_spread_over_the_buckets() {
		// Every 3-letter lowercase prefix: their low bits pick the bucket and must not pile up
		let mut keys = Vec::new();
		for a in 'a'..='z' {
			for b in 'a'..='z' {
				for c in 'a'..='z' {
					keys.push(format!("{a}{b}{c}"));
				}
			}
		}
		let buckets = 1 << 12;
		let mut counts = vec![0usize; buckets];
		for key in &keys {
			counts[(hash_of(key) as usize) & (buckets - 1)] += 1;
		}
		let used = counts.iter().filter(|count| **count > 0).count();
		// 17 576 keys in 4 096 buckets: a uniform hash leaves almost no bucket empty
		assert!(used > buckets * 9 / 10, "only {used} buckets used out of {buckets}");
		assert!(counts.iter().max().unwrap() < &20, "a bucket holds {} keys", counts.iter().max().unwrap());
	}

	#[test]
	fn tables_work_with_the_fast_hasher() {
		let mut map: FastHashMap<Box<str>, u32> = FastHashMap::default();
		map.insert("ab".into(), 1);
		map.insert("été".into(), 2);
		assert_eq!(map.get("ab"), Some(&1));
		assert_eq!(map.get("été"), Some(&2));
		assert_eq!(map.get("x"), None);
	}
}
