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

