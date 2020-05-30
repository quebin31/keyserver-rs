use std::sync::Arc;

use prost::Message as PMessage;
use rocksdb::{Error as RocksError, Options, DB};

use crate::models::{keyserver::Peers, database::DatabaseWrapper};

const METADATA_NAMESPACE: u8 = b'm';
const PEER_NAMESPACE: u8 = b'p';

#[derive(Clone)]
pub struct Database(Arc<DB>);

impl Database {
    pub fn try_new(path: &str) -> Result<Self, RocksError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);

        DB::open(&opts, &path).map(Arc::new).map(Database)
    }

    pub fn get_raw_metadata(&self, addr: &[u8]) -> Result<Option<Vec<u8>>, RocksError> {
        let key = [&[METADATA_NAMESPACE], addr].concat();
        self.0.get(key)
    }

    /// Get a `DatabaseWrapper` from the database.
    pub fn get_metadata(&self, addr: &[u8]) -> Result<Option<DatabaseWrapper>, RocksError> {
        self.get_raw_metadata(addr).map(|raw_opt| {
            raw_opt.map(|raw| {
                DatabaseWrapper::decode(&raw[..]).unwrap() // This panics if stored bytes are malformed
            })
        })
    }

    /// Put a serialized `DatabaseWrapper` to the database.
    pub fn put_metadata(&self, addr: &[u8], raw: &[u8]) -> Result<(), RocksError> {
        // Prefix key
        let key = [&[METADATA_NAMESPACE], addr].concat();

        self.0.put(key, raw)
    }

    /// Get `Peers` from database.
    pub fn get_peers(&self) -> Result<Option<Peers>, RocksError> {
        self.get_peers_raw().map(|raw_peers_opt| {
            raw_peers_opt.map(|raw_metadata| {
                Peers::decode(&raw_metadata[..]).unwrap() // This panics if stored bytes are malformed
            })
        })
    }

    /// Get serialized `Peers` from database.
    pub fn get_peers_raw(&self) -> Result<Option<Vec<u8>>, RocksError> {
        self.0.get([PEER_NAMESPACE])
    }

    /// Put serialized `Peers` to database.
    pub fn put_peers(&self, raw: &[u8]) -> Result<(), RocksError> {
        self.0.put([PEER_NAMESPACE], raw)
    }
}
