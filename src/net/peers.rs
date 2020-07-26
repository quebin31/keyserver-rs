use thiserror::Error;
use warp::{http::Response, hyper::Body, reject::Reject};

use super::IntoResponse;
use crate::{peering::PeerHandler, SETTINGS};

#[derive(Debug, Error)]
#[error("peering not supported")]
pub struct PeeringUnavailible;

impl Reject for PeeringUnavailible {}

impl IntoResponse for PeeringUnavailible {
    fn to_status(&self) -> u16 {
        501
    }
}

pub async fn get_peers<S: Clone>(
    peer_handler: PeerHandler<S>,
) -> Result<Response<Body>, PeeringUnavailible> {
    if !SETTINGS.peering.enabled {
        return Err(PeeringUnavailible);
    }

    let raw_peers = peer_handler.get_raw_peers().await;
    Ok(Response::builder().body(Body::from(raw_peers)).unwrap()) // TODO: Headers
}
