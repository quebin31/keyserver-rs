use std::{fmt, sync::Arc};

use rocksdb::Error as RocksError;
use tokio::sync::RwLock;
use warp::{http::Response, hyper::Body, reject::Reject};

use super::IntoResponse;
use crate::{peering::PeerState, SETTINGS};

#[derive(Debug)]
pub enum PeerError {
    Database(RocksError),
    Unavailable,
}

impl fmt::Display for PeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            Self::Database(err) => return err.fmt(f),
            Self::Unavailable => "peering not supported",
        };
        f.write_str(printable)
    }
}

impl From<RocksError> for PeerError {
    fn from(err: RocksError) -> Self {
        PeerError::Database(err)
    }
}

impl Reject for PeerError {}

impl IntoResponse for PeerError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Database(_) => 500,
            Self::Unavailable => 501,
        }
    }
}

pub async fn get_peers<C>(
    peer_state: Arc<RwLock<PeerState<C>>>,
) -> Result<Response<Body>, PeerError> {
    if SETTINGS.peering.enabled {
        return Err(PeerError::Unavailable);
    }

    let raw_peers = peer_state.read().await.get_raw_peers();
    Ok(Response::builder().body(Body::from(raw_peers)).unwrap()) // TODO: Headers
}
