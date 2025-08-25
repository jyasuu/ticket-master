use crate::Result;
use rocksdb::{DB, Options};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// RocksDB-based state store for persistent storage
pub struct RocksDBStore {
    db: DB,
}

impl RocksDBStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_open_files(10000);
        opts.set_use_fsync(false);
        opts.set_bytes_per_sync(8388608);
        opts.optimize_for_point_lookup(1024);
        opts.set_table_cache_num_shard_bits(6);
        opts.set_max_write_buffer_number(32);
        opts.set_write_buffer_size(536870912);
        opts.set_target_file_size_base(1073741824);
        opts.set_min_write_buffer_number_to_merge(4);
        opts.set_level_zero_stop_writes_trigger(2000);
        opts.set_level_zero_slowdown_writes_trigger(0);
        opts.set_compaction_style(rocksdb::DBCompactionStyle::Universal);

        let db = DB::open(&opts, path)?;
        Ok(Self { db })
    }

    pub fn get<T>(&self, key: &str) -> Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        match self.db.get(key)? {
            Some(value) => {
                let deserialized: T = serde_json::from_slice(&value)?;
                Ok(Some(deserialized))
            }
            None => Ok(None),
        }
    }

    pub fn put<T>(&self, key: &str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let serialized = serde_json::to_vec(value)?;
        self.db.put(key, serialized)?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        self.db.delete(key)?;
        Ok(())
    }

    pub fn contains_key(&self, key: &str) -> Result<bool> {
        Ok(self.db.get(key)?.is_some())
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

impl Drop for RocksDBStore {
    fn drop(&mut self) {
        let _ = self.db.flush();
    }
}