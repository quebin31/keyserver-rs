mod token_cache;

pub use token_cache::*;

use std::{fmt, sync::Arc};

use cashweb::keyserver_client::{
    services::{GetPeersError, SampleError},
    KeyserverManager,
};
use hyper::{
    client::{Client as HttpClient, HttpConnector},
    Body, Request, Response, Uri,
};
use prost::Message as _;
use rocksdb::Error as RocksError;
use tokio::sync::RwLock;
use tower_service::Service;
use tracing::warn;

use crate::{
    db::Database,
    models::keyserver::{Peer, Peers},
};

pub fn parse_uri_warn(uri_str: &str) -> Option<Uri> {
    let uri = uri_str.parse();
    match uri {
        Ok(some) => Some(some),
        Err(err) => {
            warn!(message = "uri parsing failed", error=%err, uri = %uri_str);
            None
        }
    }
}

#[derive(Clone)]
pub struct PeerHandler<S> {
    keyserver_manager: KeyserverManager<S>,
    peers_cache: Arc<RwLock<Vec<u8>>>,
}

fn uris_to_peers(uris: &[Uri]) -> Peers {
    let peers = uris
        .iter()
        .map(|uri| uri.to_string())
        .map(|uri_str| Peer { url: uri_str })
        .collect();
    Peers { peers }
}

fn uris_to_raw_peers(uris: &[Uri]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(uris.len());
    let peers = uris_to_peers(uris);
    peers.encode(&mut buffer).unwrap(); // This is safe
    buffer
}

impl PeerHandler<HttpClient<HttpConnector>> {
    /// Construct new [`PeerHandler`].
    pub fn new(uris: Vec<Uri>) -> Self {
        let http_client = HttpClient::new();
        let peers_cache = Arc::new(RwLock::new(uris_to_raw_peers(&uris)));
        let keyserver_manager = KeyserverManager::from_service(http_client, uris);
        Self {
            keyserver_manager,
            peers_cache,
        }
    }
}

impl<S> PeerHandler<S>
where
    S: Clone,
{
    pub fn get_keyserver_manager(&self) -> &KeyserverManager<S> {
        &self.keyserver_manager
    }

    pub async fn get_urls(&self) -> Vec<Uri> {
        self.keyserver_manager.get_uris().read().await.clone()
    }

    pub async fn set_peers(&self, uris: Vec<Uri>) {
        let mut peer_cache_write = self.peers_cache.write().await;
        let uris_shared = self.keyserver_manager.get_uris();
        let mut uris_write = uris_shared.write().await;
        *peer_cache_write = uris_to_raw_peers(&uris);
        *uris_write = uris;
    }

    pub async fn get_raw_peers(&self) -> Vec<u8> {
        self.peers_cache.read().await.clone()
    }

    pub async fn persist(&self, database: &Database) -> Result<(), RocksError> {
        let raw_peers = self.get_raw_peers().await;
        database.put_peers(&raw_peers)
    }
}

impl<S> PeerHandler<S>
where
    S: Service<Request<Body>, Response = Response<Body>>,
    S: Send + Clone + 'static,
    S::Future: Send,
    S::Error: fmt::Debug + Send + fmt::Display,
{
    pub async fn inflate(&self) -> Result<(), SampleError<GetPeersError<S::Error>>> {
        // Crawl peers, collecting Peers
        let aggregate_response = self.get_keyserver_manager().crawl_peers().await?;
        // TODO: Ban misbehaviour

        // Collect URIs
        let uris = aggregate_response
            .response
            .peers
            .into_iter()
            .filter_map(|peer| parse_uri_warn(&peer.url))
            .collect();
        self.set_peers(uris).await;
        Ok(())
    }
}
