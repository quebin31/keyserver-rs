use std::{fmt, sync::Arc};

use bitcoincash_addr::Address;
use bytes::Bytes;
use cashweb::bitcoin_client::HttpConnector;
use cashweb::token::{extract_pop, schemes::chain_commitment::*};
use http::header::HeaderMap;
use log::info;
use prost::Message as _;
use sha2::{Digest, Sha256};
use warp::{http::Response, hyper::Body, reject::Reject};

use crate::{models::wrapper::AuthWrapper, net::payments};

#[derive(Debug)]
pub enum ProtectionError {
    MissingToken(Vec<u8>, Vec<u8>),
    Validation(ValidationError),
    Decode(prost::DecodeError),
}

impl fmt::Display for ProtectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken(_, _) => f.write_str("missing token"),
            Self::Validation(err) => err.fmt(f),
            Self::Decode(err) => err.fmt(f),
        }
    }
}

pub async fn protection_error_recovery(err: &ProtectionError) -> Response<Body> {
    match err {
        ProtectionError::Validation(_) => Response::builder()
            .status(400)
            .body(Body::from(err.to_string()))
            .unwrap(),
        ProtectionError::MissingToken(pubkey_digest, metadata_digest) => {
            payments::construct_payment_response(pubkey_digest, metadata_digest)
        }
        ProtectionError::Decode(err) => Response::builder()
            .status(400)
            .body(Body::from(err.to_string()))
            .unwrap(),
    }
}

impl Reject for ProtectionError {}

pub async fn pop_protection(
    addr: Address,
    auth_wrapper_raw: Bytes,
    header_map: HeaderMap,
    token_scheme: Arc<ChainCommitmentScheme<HttpConnector>>,
) -> Result<(Address, Bytes, AuthWrapper, Vec<u8>), ProtectionError> {
    let auth_wrapper =
        AuthWrapper::decode(auth_wrapper_raw.clone()).map_err(ProtectionError::Decode)?;

    let metadata_hash = if auth_wrapper.payload_digest.len() == 32 {
        auth_wrapper.payload_digest.clone()
    } else {
        Sha256::digest(&auth_wrapper_raw[..]).to_vec()
    };

    // SHA256 of the public key
    let pub_key_hash = Sha256::digest(&auth_wrapper.pub_key);

    match extract_pop(&header_map) {
        Some(pop_token) => {
            info!("found token {}", pop_token);
            let raw_token = token_scheme
                .validate_token(&pub_key_hash, &metadata_hash, pop_token)
                .await
                .map_err(ProtectionError::Validation)?;
            Ok((addr, auth_wrapper_raw, auth_wrapper, raw_token))
        }
        None => Err(ProtectionError::MissingToken(
            pub_key_hash.to_vec(),
            metadata_hash.to_vec(),
        )),
    }
}
