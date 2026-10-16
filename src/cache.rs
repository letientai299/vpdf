use std::collections::HashMap;
use std::hash::Hash;

struct Entry<V> {
  value: V,
  weight: usize,
  touched: u64,
}

pub struct Cache<K, V> {
  entries: HashMap<K, Entry<V>>,
  budget: usize,
  used: usize,
  clock: u64,
}

impl<K: Eq + Hash + Clone, V> Cache<K, V> {
  pub fn new(budget: usize) -> Self {
    Self { entries: HashMap::new(), budget, used: 0, clock: 0 }
  }

  pub fn get(&mut self, key: &K) -> Option<&V> {
    self.clock += 1;
    let entry = self.entries.get_mut(key)?;
    entry.touched = self.clock;
    Some(&entry.value)
  }

  pub fn insert(&mut self, key: K, value: V, weight: usize) -> Vec<K> {
    let mut evicted = Vec::new();
    if let Some(old) = self.entries.remove(&key) {
      self.used -= old.weight;
    }
    if weight > self.budget {
      evicted.push(key);
      return evicted;
    }
    while self.used + weight > self.budget {
      let oldest = self.entries.iter().min_by_key(|(_, e)| e.touched);
      let Some((key, _)) = oldest else { break };
      let key = key.clone();
      if let Some(entry) = self.entries.remove(&key) {
        self.used -= entry.weight;
      }
      evicted.push(key);
    }
    self.clock += 1;
    self.used += weight;
    self.entries.insert(key, Entry { value, weight, touched: self.clock });
    evicted
  }

  pub fn used(&self) -> usize {
    self.used
  }

  pub fn clear(&mut self) {
    self.entries.clear();
    self.used = 0;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn evicts_lru_and_bounds_oversized() {
    let mut cache = Cache::new(10);
    cache.insert(1, "first", 5);
    cache.insert(2, "second", 5);
    assert_eq!(cache.get(&1), Some(&"first"));
    assert_eq!(cache.insert(3, "third", 5), vec![2]);
    assert_eq!(cache.insert(4, "large", 11), vec![4]);
    assert!(cache.used() <= 10);
    assert_eq!(cache.get(&4), None);
  }
}
