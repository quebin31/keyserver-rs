use std::{collections::HashSet, convert::TryFrom};

use http::uri::InvalidUri;
use hyper::{body::aggregate, client::connect::Connect, Body, Client, Error as HyperError, Uri};
use prost::{DecodeError, Message as _};
use rocksdb::Error as RocksError;

use crate::{
    db::Database,
    models::address_metadata::{Peer, Peers},
    SETTINGS,
};

pub struct PeerState<C> {
    peers: Peers,
    client: PeeringClient<C>,
}

impl<C> PeerState<C>
where
    C: Clone + Send + Sync,
    C: Connect + 'static,
{
    pub fn new(peers: Peers, connector: C) -> Self {
        Self {
            peers,
            client: PeeringClient {
                client: Client::builder().build::<_, Body>(connector),
            },
        }
    }

    pub async fn traverse(&self) -> Peers {
        // TODO: Remove clones
        let mut current_urls = self.get_url_set();
        let mut new_urls = current_urls.clone();

        loop {
            // Fan out and find peers
            let found_urls = self.client.get_fan(&new_urls).await;

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

        Peers {
            peers: current_urls
                .into_iter()
                .map(move |url| Peer { url })
                .collect(),
        }
    }
}

impl<C> PeerState<C> {
    pub fn get_url_set(&self) -> HashSet<String> {
        self.peers
            .peers
            .clone()
            .into_iter()
            .map(move |peer| peer.url)
            .collect()
    }

    pub fn set_peers(&mut self, peers: Peers) {
        self.peers = peers
    }

    pub fn get_raw_peers(&self) -> Vec<u8> {
        // Serialize peers
        let mut raw_peers = Vec::with_capacity(self.peers.encoded_len());
        self.peers.encode(&mut raw_peers).unwrap();
        raw_peers
    }

    pub fn persist(&self, database: &Database) -> Result<(), RocksError> {
        let raw_peers = self.get_raw_peers();
        database.put_peers(&raw_peers)
    }
}

pub struct PeeringClient<C> {
    client: Client<C, Body>,
}

pub enum PeerError {
    Hyper(HyperError),
    Decode(DecodeError),
    Uri(InvalidUri),
}

impl From<HyperError> for PeerError {
    fn from(err: HyperError) -> Self {
        Self::Hyper(err)
    }
}

impl<C> PeeringClient<C>
where
    C: Clone + Send + Sync,
    C: Connect + 'static,
{
    pub async fn get_peers(&self, url: &str) -> Result<Vec<String>, PeerError> {
        let uri = Uri::try_from(url).map_err(PeerError::Uri)?;
        let response = self.client.get(uri).await?;
        let raw = aggregate(response.into_body()).await?;
        let peers = Peers::decode(raw).map_err(PeerError::Decode)?;
        Ok(peers.peers.into_iter().map(|peer| peer.url).collect())
    }

    pub async fn get_fan(&self, url_set: &HashSet<String>) -> HashSet<String> {
        let fan = url_set.into_iter().map(|url| self.get_peers(url));
        let new_urls: HashSet<String> = futures::future::join_all(fan)
            .await
            .into_iter()
            .filter_map(|urls| urls.ok())
            .flatten()
            .collect();
        new_urls
    }
}
