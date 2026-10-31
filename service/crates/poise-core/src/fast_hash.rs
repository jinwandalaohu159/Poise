//! 调度用的哈希表：键是进程号、进程身份这类系统给的数，每轮要查一两万次。标准库默认的哈希为了防止
//! 有人故意制造冲突，每次要算一大串；这里的键没人能控制，用一个简单快速的哈希（和 rustc 内部用的一样），
//! 查找快三四倍。遍历顺序本来就不保证，凡是顺序会影响结果的地方都已排序。

use std::hash::{BuildHasherDefault, Hasher};

pub type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FastHasher>>;
pub type HashSet<T> = std::collections::HashSet<T, BuildHasherDefault<FastHasher>>;

/// 乘法散列（FxHash）：每 8 个字节一步。
#[derive(Default, Clone, Copy)]
pub struct FastHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("8 字节")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(last));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_as_a_map_and_spreads_nearby_keys() {
        let mut m: HashMap<u32, &str> = HashMap::default();
        m.insert(4, "a");
        m.insert(8, "b");
        assert_eq!(m.get(&4), Some(&"a"));
        let hash = |k: u32| {
            let mut h = FastHasher::default();
            h.write_u32(k);
            h.finish()
        };
        assert_ne!(hash(4), hash(8));
        let mut s: HashSet<String> = HashSet::default();
        s.insert("chrome.exe".into());
        assert!(s.contains("chrome.exe"));
    }
}
