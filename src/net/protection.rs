use std::{fmt, sync::Arc};

use bitcoincash_addr::Address;
use bytes::Bytes;
use cashweb::bitcoin_client::HttpConnector;
use cashweb::token::{extract_pop, schemes::chain_commitment::*};
use http::header::HeaderMap;
use sha2::{Digest, Sha256};
use warp::{http::Response, hyper::Body, reject::Reject};

#[derive(Debug)]
pub enum ProtectionError {
    MissingToken(Vec<u8>, Vec<u8>),
    Validation(ValidationError),
}

impl fmt::Display for ProtectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken(_, _) => f.write_str("missing token"),
            Self::Validation(err) => err.fmt(f),
        }
    }
}

pub async fn protection_error_recovery(err: &ProtectionError) -> Response<Body> {
    match err {
        ProtectionError::Validation(_) => Response::builder()
            .status(400)
            .body(Body::from(err.to_string()))
            .unwrap(),
        ProtectionError::MissingToken(_, _) => Response::builder()
            .status(400)
            .body(Body::from(err.to_string()))
            .unwrap(), // TODO: Recovery here
    }
}

impl Reject for ProtectionError {}

pub async fn pop_protection(
    addr: Address,
    metadata: Bytes,
    header_map: HeaderMap,
    token_scheme: Arc<ChainCommitmentScheme<HttpConnector>>,
) -> Result<(Address, Bytes), ProtectionError> {
    let pub_key_hash = addr.as_body();
    let metadata_hash = Sha256::digest(&metadata);

    match extract_pop(&header_map) {
        Some(pop_token) => {
            token_scheme
                .validate_token(pub_key_hash, &metadata_hash, pop_token)
                .await
                .map_err(ProtectionError::Validation)?;
            Ok((addr, metadata))
        }
        None => Err(ProtectionError::MissingToken(
            pub_key_hash.to_vec(),
            metadata_hash.to_vec(),
        )),
    }
}
