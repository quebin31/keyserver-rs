use std::sync::Arc;

use bitcoincash_addr::Address;
use bytes::Bytes;
use cashweb::bitcoin_client::HttpClient;
use cashweb::token::{extract_pop, schemes::chain_commitment::*};
use http::header::HeaderMap;
use hyper::Error as HyperError;
use prost::Message as _;
use ring::digest::{digest, SHA256};
use thiserror::Error;
use tracing::info;
use warp::{http::Response, hyper::Body, reject::Reject};

use crate::{models::wrapper::AuthWrapper, net::payments};

#[derive(Debug, Error)]
pub enum ProtectionError {
    #[error("missing token, pubkey: {0:?}")] // TODO: Make this prettier
    MissingToken(Vec<u8>, Vec<u8>),
    #[error("validation failed: {0}")]
    Validation(ValidationError<HyperError>),
    #[error("failed to decode authorization wrapper: {0}")]
    Decode(prost::DecodeError),
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
    token_scheme: Arc<ChainCommitmentScheme<HttpClient>>,
) -> Result<(Address, Bytes, AuthWrapper, Vec<u8>), ProtectionError> {
    let auth_wrapper =
        AuthWrapper::decode(auth_wrapper_raw.clone()).map_err(ProtectionError::Decode)?;

    let metadata_hash = if auth_wrapper.payload_digest.len() == 32 {
        auth_wrapper.payload_digest.clone()
    } else {
        digest(&SHA256, &auth_wrapper.payload).as_ref().to_vec()
    };

    // SHA256 of the public key
    let pub_key_hash = digest(&SHA256, &auth_wrapper.public_key);

    match extract_pop(&header_map) {
        Some(pop_token) => {
            info!(message = "found token", token = %pop_token);
            let raw_token = token_scheme
                .validate_token(pub_key_hash.as_ref(), &metadata_hash, pop_token)
                .await
                .map_err(ProtectionError::Validation)?;
            Ok((addr, auth_wrapper_raw, auth_wrapper, raw_token))
        }
        None => Err(ProtectionError::MissingToken(
            pub_key_hash.as_ref().to_vec(),
            metadata_hash,
        )),
    }
}
