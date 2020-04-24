use std::{
    collections::{HashSet, VecDeque},
    convert::TryFrom,
};

use bytes::Bytes;
use dashmap::DashMap;
use http::{header::AUTHORIZATION, uri::InvalidUri};
use hyper::{
    body::aggregate, client::connect::Connect, Body, Client, Error as HyperError, Request, Uri,
};
use prost::{DecodeError, Message as _};
use rand::{seq::SliceRandom, thread_rng};
use rocksdb::Error as RocksError;
use tokio::sync::RwLock;

use crate::{
    db::Database,
    models::keyserver::{Peer, Peers},
    SETTINGS,
};

const BLOCK_BUFFER: usize = 8;
const SAMPLE_SIZE: usize = 4;

pub struct TokenCache {
    tokens_blocks: RwLock<VecDeque<DashMap<Vec<u8>, String>>>,
    db: Database,
}

impl TokenCache {
    pub fn new(db: Database) -> Self {
        let deque = VecDeque::from(vec![Default::default(); BLOCK_BUFFER]);
        Self {
            tokens_blocks: RwLock::new(deque),
            db,
        }
    }

    pub async fn add_token(&self, addr: Vec<u8>, token: String) {
        let token_blocks = self.tokens_blocks.read().await;
        // TODO: Check previous blocks?
        // TODO: Check consistency garauntees of the dashmap under iter + insert
        token_blocks.front().unwrap().insert(addr, token); // TODO: Double check this is safe
    }

    pub async fn broadcast_block<C>(&self, peers: &Peers, client: PeeringClient<C>)
    where
        C: Clone + Send + Sync,
        C: Connect + 'static,
    {
        let mut token_blocks = self.tokens_blocks.write().await;

        // Cycle blocks
        token_blocks.push_front(Default::default());
        let token_block = match token_blocks.pop_back() {
            Some(some) => some,
            None => return,
        };

        // Sample peers
        let mut rng = thread_rng();
        let peer_choices: Vec<_> = peers
            .peers
            .choose_multiple(&mut rng, SAMPLE_SIZE)
            .cloned()
            .collect();

        for (addr, token) in token_block.into_iter() {
            let metadata = match self.db.get_raw_metadata(&addr) {
                Ok(Some(some)) => some,
                _ => continue,
            };
            let metadata = Bytes::from(metadata);
            for peer in &peer_choices {
                client
                    .put_metadata(&peer.url, metadata.clone(), &token)
                    .await;
                // TODO: Make this non-blocking
                // TODO: Error handling -> remove as peer
            }
        }
    }
}

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
        let fan = url_set.iter().map(|url| self.get_peers(url));
        let new_urls: HashSet<String> = futures::future::join_all(fan)
            .await
            .into_iter()
            .filter_map(|urls| urls.ok())
            .flatten()
            .collect();
        new_urls
    }

    pub async fn put_metadata(
        &self,
        url: &str,
        metadata: Bytes,
        token: &str,
    ) -> Result<(), HyperError> {
        let uri = Uri::try_from(url).unwrap(); // TODO: Make this safe
        let request = Request::put(uri)
            .header(AUTHORIZATION, token)
            .body(Body::from(metadata))
            .unwrap();
        self.client.request(request).await?;
        Ok(())
    }
}
