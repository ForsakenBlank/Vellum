//! a least recently used cache with a byte budget

use std::collections::HashMap;

pub struct Lru<V> {
    map: HashMap<u64, Entry<V>>,
    budget: u64,
    used: u64,
    tick: u64,
}

struct Entry<V> {
    value: V,
    bytes: u64,
    used_at: u64,
}

impl<V: Clone> Lru<V> {
    pub fn new(budget: u64) -> Self {
        Self {
            map: HashMap::new(),
            budget,
            used: 0,
            tick: 0,
        }
    }

    pub fn get(&mut self, key: u64) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(&key).map(|e| {
            e.used_at = tick;
            e.value.clone()
        })
    }

    pub fn contains(&self, key: u64) -> bool {
        self.map.contains_key(&key)
    }

    /// stores a value. something bigger than the whole budget is not kept.
    pub fn put(&mut self, key: u64, value: V, bytes: u64) {
        if bytes > self.budget {
            return;
        }
        self.tick += 1;
        if let Some(old) = self.map.insert(
            key,
            Entry {
                value,
                bytes,
                used_at: self.tick,
            },
        ) {
            self.used -= old.bytes;
        }
        self.used += bytes;
        self.trim();
    }

    fn trim(&mut self) {
        if self.used <= self.budget {
            return;
        }
        let mut entries: Vec<(u64, u64)> = self.map.iter().map(|(k, e)| (e.used_at, *k)).collect();
        entries.sort_unstable();
        for (_, key) in entries {
            if self.used <= self.budget * 9 / 10 {
                break;
            }
            if let Some(e) = self.map.remove(&key) {
                self.used -= e.bytes;
            }
        }
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.trim();
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_first() {
        let mut c = Lru::new(100);
        c.put(1, "a", 40);
        c.put(2, "b", 40);
        assert_eq!(c.get(1), Some("a"));
        c.put(3, "c", 40);
        assert!(c.contains(1) && c.contains(3) && !c.contains(2));
        assert!(c.used() <= 100);
    }

    #[test]
    fn refuses_values_bigger_than_the_budget() {
        let mut c = Lru::new(10);
        c.put(1, 1, 11);
        assert!(c.is_empty());
        c.put(1, 1, 5);
        c.put(1, 2, 6);
        assert_eq!(c.used(), 6);
        assert_eq!(c.get(1), Some(2));
    }
}
