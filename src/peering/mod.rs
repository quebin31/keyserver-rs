mod client;
mod token_cache;

pub use client::*;
pub use token_cache::*;

use std::{collections::HashSet, sync::Arc};

use bytes::Buf;
use hyper::client::connect::Connect;
use prost::Message as _;
use rand::{rngs::OsRng, seq::IteratorRandom};
use rocksdb::Error as RocksError;
use tokio::sync::RwLock;

use crate::{
    db::Database,
    models::keyserver::{Peer, Peers},
    SETTINGS,
};

#[derive(Clone)]
pub struct PeerHandler<C> {
    peers: Arc<RwLock<HashSet<String>>>,
    client: PeeringClient<C>,
}

impl<C> PeerHandler<C>
where
    C: Clone + Send + Sync,
    C: Connect + 'static,
{
    pub fn new(peers: HashSet<String>, connector: C) -> Self {
        Self {
            peers: Arc::new(RwLock::new(peers)),
            client: PeeringClient::new(connector),
        }
    }

    pub async fn traverse(&self) -> HashSet<String> {
        // TODO: Remove clones
        let mut current_urls = self.get_urls().await;
        let mut new_urls = current_urls.clone();

        loop {
            // Fan out and find peers
            let found_urls = self.client.get_peer_fan(&new_urls).await;

            // New distinct URLs
            new_urls = found_urls.difference(&current_urls).cloned().collect();

            let new_size = current_urls.len() + current_urls.len();
            if new_urls.is_empty() {
                // If no new URLs then stop
                break;
            } else if new_size > SETTINGS.peering.max_peers as usize {
                // If reached maximum then stop
                new_urls
                    .into_iter()
                    .take(new_size - SETTINGS.peering.max_peers as usize)
                    .for_each(|url| {
                        current_urls.insert(url);
                    });
                break;
            } else {
                // Add new urls
                current_urls = current_urls.union(&new_urls).cloned().collect();
            }
        }

        current_urls
    }

    pub async fn sample_peer_metadata(&self, addr: &str) -> Result<Vec<u8>, PeerError> {
        let sample = self.sample().await;
        let mut metadata_fan = self.client.get_metadata_fan(addr, &sample).await;
        let raw_metadata = metadata_fan.pop().ok_or(PeerError::NotFound)?;
        // let metadata = AddressMetadata::decode(raw_metadata).map_err(PeerError::Decode)?;
        Ok(raw_metadata.bytes().to_vec())
    }

    pub async fn sample(&self) -> Vec<String> {
        // Sample peers
        self.peers
            .read()
            .await
            .iter()
            .choose_multiple(&mut OsRng, SETTINGS.peering.fan_size)
            .iter()
            .map(|url| url.to_string())
            .collect()
    }
}

impl<C> PeerHandler<C> {
    pub async fn get_urls(&self) -> HashSet<String> {
        self.peers.read().await.clone()
    }

    pub async fn set_peers(&mut self, peers: HashSet<String>) {
        *self.peers.write().await = peers;
    }

    pub async fn get_raw_peers(&self) -> Vec<u8> {
        // Serialize peers
        let peers = Peers {
            peers: self
                .peers
                .read()
                .await
                .iter()
                .map(|uri| Peer {
                    url: uri.to_string(),
                })
                .collect(),
        };
        let mut raw_peers = Vec::with_capacity(peers.encoded_len());
        peers.encode(&mut raw_peers).unwrap();
        raw_peers
    }

    pub async fn persist(&self, database: &Database) -> Result<(), RocksError> {
        let raw_peers = self.get_raw_peers().await;
        database.put_peers(&raw_peers)
    }
}
