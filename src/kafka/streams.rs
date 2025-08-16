use crate::Result;
use dashmap::DashMap;
use std::sync::Arc;

// Simple in-memory state store implementation
// In a production system, you'd want to use RocksDB or similar
pub struct StateStore<K, V> {
    data: Arc<DashMap<K, V>>,
}

impl<K, V> StateStore<K, V>
where
    K: std::hash::Hash + Eq + Clone,
    V: Clone,
{
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.data.get(key).map(|entry| entry.value().clone())
    }

    pub fn put(&self, key: K, value: V) {
        self.data.insert(key, value);
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        self.data.remove(key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.data.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl<K, V> Clone for StateStore<K, V> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
        }
    }
}

// Stream processor trait for handling messages
pub trait StreamProcessor<K, V, R> {
    fn process(&self, key: K, value: V) -> Result<Option<(K, R)>>;
}

// Simple stream processing context
pub struct ProcessingContext {
    pub stores: DashMap<String, Box<dyn std::any::Any + Send + Sync>>,
}

impl ProcessingContext {
    pub fn new() -> Self {
        Self {
            stores: DashMap::new(),
        }
    }

    pub fn add_store<K, V>(&self, name: String, store: StateStore<K, V>)
    where
        K: 'static + Send + Sync,
        V: 'static + Send + Sync,
    {
        self.stores.insert(name, Box::new(store));
    }

    pub fn get_store<K, V>(&self, name: &str) -> Option<StateStore<K, V>>
    where
        K: 'static + Send + Sync + Clone,
        V: 'static + Send + Sync + Clone,
    {
        self.stores.get(name).and_then(|entry| {
            entry.value().downcast_ref::<StateStore<K, V>>().cloned()
        })
    }
}