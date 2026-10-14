//! Version vector (vector clock) for distributed file version tracking.
//! Ensures causal ordering and conflict detection independent of wall-clock time.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorRelation {
    Equal,
    /// Self strictly dominates Other (Self is newer)
    Newer,
    /// Other strictly dominates Self (Self is older)
    Older,
    /// Neither dominates; concurrent modifications detected (conflict)
    Concurrent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector {
    pub counters: BTreeMap<String, u64>,
}

impl VersionVector {
    pub fn new() -> Self {
        Self {
            counters: BTreeMap::new(),
        }
    }

    pub fn from_device(device_id: &str, counter: u64) -> Self {
        let mut vv = Self::new();
        if counter > 0 {
            vv.counters.insert(device_id.to_string(), counter);
        }
        vv
    }

    pub fn is_empty(&self) -> bool {
        self.counters.is_empty() || self.counters.values().all(|&c| c == 0)
    }

    pub fn get(&self, device_id: &str) -> u64 {
        self.counters.get(device_id).copied().unwrap_or(0)
    }

    pub fn set(&mut self, device_id: &str, counter: u64) {
        if counter == 0 {
            self.counters.remove(device_id);
        } else {
            self.counters.insert(device_id.to_string(), counter);
        }
    }

    pub fn increment(&mut self, device_id: &str) -> u64 {
        let counter = self.counters.entry(device_id.to_string()).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Merges another version vector into self by taking the maximum counter for each device.
    pub fn merge(&mut self, other: &VersionVector) {
        for (dev, &cnt) in &other.counters {
            let entry = self.counters.entry(dev.clone()).or_insert(0);
            if cnt > *entry {
                *entry = cnt;
            }
        }
    }

    /// Determines the causal relationship between self and other.
    pub fn relation(&self, other: &VersionVector) -> VectorRelation {
        let mut has_greater = false;
        let mut has_lesser = false;

        let all_keys: BTreeSet<&String> = self.counters.keys().chain(other.counters.keys()).collect();
        for key in all_keys {
            let c1 = self.get(key);
            let c2 = other.get(key);
            if c1 > c2 {
                has_greater = true;
            } else if c1 < c2 {
                has_lesser = true;
            }
        }

        match (has_greater, has_lesser) {
            (false, false) => VectorRelation::Equal,
            (true, false) => VectorRelation::Newer,
            (false, true) => VectorRelation::Older,
            (true, true) => VectorRelation::Concurrent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn empty_vectors_are_equal() {
        let v1 = VersionVector::new();
        let v2 = VersionVector::new();
        assert_eq!(v1.relation(&v2), VectorRelation::Equal);
    }

    #[test]
    fn single_device_ordering() {
        let mut v1 = VersionVector::new();
        v1.increment("mac");

        let mut v2 = VersionVector::new();
        v2.increment("mac");
        v2.increment("mac");

        assert_eq!(v1.relation(&v2), VectorRelation::Older);
        assert_eq!(v2.relation(&v1), VectorRelation::Newer);
    }

    #[test]
    fn concurrent_modifications() {
        let mut v1 = VersionVector::new();
        v1.increment("mac");

        let mut v2 = VersionVector::new();
        v2.increment("win");

        assert_eq!(v1.relation(&v2), VectorRelation::Concurrent);
        assert_eq!(v2.relation(&v1), VectorRelation::Concurrent);
    }

    #[test]
    fn merge_takes_pairwise_max() {
        let mut v1 = VersionVector::new();
        v1.set("dev_a", 3);
        v1.set("dev_b", 1);

        let mut v2 = VersionVector::new();
        v2.set("dev_a", 2);
        v2.set("dev_b", 5);
        v2.set("dev_c", 4);

        v1.merge(&v2);
        assert_eq!(v1.get("dev_a"), 3);
        assert_eq!(v1.get("dev_b"), 5);
        assert_eq!(v1.get("dev_c"), 4);
        assert_eq!(v1.relation(&v2), VectorRelation::Newer);
    }

    #[test]
    fn randomized_property_test_invariants() {
        let mut rng = rand::thread_rng();
        let devices = ["dev_a", "dev_b", "dev_c", "dev_d"];

        for _ in 0..2_000 {
            let mut v1 = VersionVector::new();
            let mut v2 = VersionVector::new();
            let mut v3 = VersionVector::new();

            for dev in devices {
                if rng.gen_bool(0.7) {
                    v1.set(dev, rng.gen_range(0..10));
                }
                if rng.gen_bool(0.7) {
                    v2.set(dev, rng.gen_range(0..10));
                }
                if rng.gen_bool(0.7) {
                    v3.set(dev, rng.gen_range(0..10));
                }
            }

            // Invariant 1: Reflexivity
            assert_eq!(v1.relation(&v1), VectorRelation::Equal);

            // Invariant 2: Asymmetry
            let rel_12 = v1.relation(&v2);
            let rel_21 = v2.relation(&v1);
            match rel_12 {
                VectorRelation::Equal => assert_eq!(rel_21, VectorRelation::Equal),
                VectorRelation::Newer => assert_eq!(rel_21, VectorRelation::Older),
                VectorRelation::Older => assert_eq!(rel_21, VectorRelation::Newer),
                VectorRelation::Concurrent => assert_eq!(rel_21, VectorRelation::Concurrent),
            }

            // Invariant 3: Transitivity
            if rel_12 == VectorRelation::Newer && v2.relation(&v3) == VectorRelation::Newer {
                assert_eq!(v1.relation(&v3), VectorRelation::Newer);
            }

            // Invariant 4: Merge dominance
            let mut merged = v1.clone();
            merged.merge(&v2);
            assert!(matches!(merged.relation(&v1), VectorRelation::Equal | VectorRelation::Newer));
            assert!(matches!(merged.relation(&v2), VectorRelation::Equal | VectorRelation::Newer));
        }
    }
}
