use std::{collections::VecDeque, fmt, sync::Arc};

use bitcoincash_addr::Address;
use dashmap::DashSet;
use hyper::{Body, Request, Response};
use tokio::sync::RwLock;
use tower_service::Service;

use super::PeerHandler;
use crate::{db::Database, SETTINGS};

#[derive(Clone)]
pub struct TokenCache {
    tokens_blocks: Arc<RwLock<VecDeque<DashSet<Address>>>>,
}

impl Default for TokenCache {
    fn default() -> Self {
        let deque = VecDeque::from(vec![Default::default(); SETTINGS.peering.broadcast_delay]);
        Self {
            tokens_blocks: Arc::new(RwLock::new(deque)),
        }
    }
}

impl TokenCache {
    pub async fn add_token(&self, addr: Address) {
        let token_blocks = self.tokens_blocks.read().await;
        // TODO: Check previous blocks?
        // TODO: Check consistency garauntees of the dashmap under iter + insert
        token_blocks.front().unwrap().insert(addr); // TODO: Double check this is safe
    }

    pub async fn broadcast_block<S>(&self, peer_handler: &PeerHandler<S>, db: &Database)
    where
        S: Service<Request<Body>, Response = Response<Body>>,
        S: Send + Clone + 'static,
        <S as Service<Request<Body>>>::Future: Send,
        S::Error: Send + fmt::Debug + fmt::Display,
    {
        let mut token_blocks = self.tokens_blocks.write().await;

        // Cycle blocks
        token_blocks.push_front(Default::default());
        let token_block = match token_blocks.pop_back() {
            Some(some) => some,
            None => return,
        };

        // Broadcast each metadata
        for addr in token_block.into_iter() {
            let db_wrapper = match db.get_metadata(addr.as_body()) {
                Ok(Some(some)) => some,
                _ => continue,
            };
            let addr_str = addr.encode().unwrap(); // This is safe

            // Reconstruct token
            let raw_token = db_wrapper.token;
            let url_safe_config = base64::Config::new(base64::CharacterSet::UrlSafe, false);
            let token = format!("POP {}", base64::encode_config(raw_token, url_safe_config));

            let _response = peer_handler
                .get_keyserver_manager()
                .uniform_broadcast_raw_metadata(
                    &addr_str,
                    db_wrapper.serialized_auth_wrapper,
                    token,
                    SETTINGS.peering.push_fan_size,
                )
                .await;

            // TODO: Remove errors from peer list
        }
    }
}
