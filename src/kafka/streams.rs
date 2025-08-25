use crate::{Result, RocksDBStore};
use dashmap::DashMap;
use std::sync::Arc;
use std::path::Path;

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

// Enhanced state store that can use either in-memory or RocksDB
pub enum StateStoreBackend<K, V> {
    InMemory(StateStore<K, V>),
    RocksDB(Arc<RocksDBStore>),
}

impl<K, V> StateStoreBackend<K, V>
where
    K: std::hash::Hash + Eq + Clone + ToString,
    V: Clone + serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    pub fn new_in_memory() -> Self {
        Self::InMemory(StateStore::new())
    }

    pub fn new_rocksdb<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = RocksDBStore::new(path)?;
        Ok(Self::RocksDB(Arc::new(store)))
    }

    pub fn get(&self, key: &K) -> Result<Option<V>> {
        match self {
            Self::InMemory(store) => Ok(store.get(key)),
            Self::RocksDB(store) => store.get(&key.to_string()),
        }
    }

    pub fn put(&self, key: K, value: V) -> Result<()> {
        match self {
            Self::InMemory(store) => {
                store.put(key, value);
                Ok(())
            }
            Self::RocksDB(store) => store.put(&key.to_string(), &value),
        }
    }

    pub fn remove(&self, key: &K) -> Result<Option<V>> {
        match self {
            Self::InMemory(store) => Ok(store.remove(key)),
            Self::RocksDB(store) => {
                let existing = store.get(&key.to_string())?;
                store.delete(&key.to_string())?;
                Ok(existing)
            }
        }
    }

    pub fn contains_key(&self, key: &K) -> Result<bool> {
        match self {
            Self::InMemory(store) => Ok(store.contains_key(key)),
            Self::RocksDB(store) => store.contains_key(&key.to_string()),
        }
    }
}

// Simple stream processing context
pub struct ProcessingContext {
    pub stores: DashMap<String, Box<dyn std::any::Any + Send + Sync>>,
    pub state_dir: String,
}

impl ProcessingContext {
    pub fn new() -> Self {
        Self {
            stores: DashMap::new(),
            state_dir: "/tmp/kafka-streams".to_string(),
        }
    }

    pub fn with_state_dir(state_dir: String) -> Self {
        Self {
            stores: DashMap::new(),
            state_dir,
        }
    }

    pub fn add_store<K, V>(&self, name: String, store: StateStore<K, V>)
    where
        K: 'static + Send + Sync,
        V: 'static + Send + Sync,
    {
        self.stores.insert(name, Box::new(store));
    }

    pub fn add_rocksdb_store(&self, name: String, store_path: &str) -> Result<()> {
        let full_path = format!("{}/{}", self.state_dir, store_path);
        std::fs::create_dir_all(&full_path)?;
        let store = RocksDBStore::new(full_path)?;
        self.stores.insert(name, Box::new(Arc::new(store)));
        Ok(())
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

    pub fn get_rocksdb_store(&self, name: &str) -> Option<Arc<RocksDBStore>> {
        self.stores.get(name).and_then(|entry| {
            entry.value().downcast_ref::<Arc<RocksDBStore>>().cloned()
        })
    }
}