use std::sync::Arc;

use prost::Message as PMessage;
use rocksdb::{Error as RocksError, Options, DB};

use crate::models::wrapper::AuthWrapper;

const METADATA_NAMESPACE: u8 = b'p';

#[derive(Clone)]
pub struct Database(Arc<DB>);

impl Database {
    pub fn try_new(path: &str) -> Result<Self, RocksError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);

        DB::open(&opts, &path).map(Arc::new).map(Database)
    }

    pub fn get_metadata(&self, addr: &[u8]) -> Result<Option<AuthWrapper>, RocksError> {
        // Prefix key
        let key = [addr, &[METADATA_NAMESPACE]].concat();

        self.0.get(key).map(|raw_metadata_opt| {
            raw_metadata_opt.map(|raw_metadata| {
                AuthWrapper::decode(&raw_metadata[..]).unwrap() // This panics if stored bytes are malformed
            })
        })
    }

    pub fn put_metadata(&self, addr: &[u8], raw_metadata: &[u8]) -> Result<(), RocksError> {
        // Prefix key
        let key = [addr, &[METADATA_NAMESPACE]].concat();

        self.0.put(key, raw_metadata)
    }
}
