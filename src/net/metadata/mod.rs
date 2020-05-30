pub mod errors;

use bitcoincash_addr::Address;
use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue, AUTHORIZATION, MAX_FORWARDS};
use hyper::client::connect::Connect;
use prost::Message as _;
use secp256k1::{key::PublicKey, Message, Secp256k1, Signature};
use sha2::{Digest, Sha256};
use tokio::task;
use warp::{http::Response, hyper::Body};

use crate::{
    db::Database, models::wrapper::AuthWrapper, peering::PeerHandler, peering::TokenCache,
};
pub use errors::*;

#[derive(Debug, Deserialize)]
pub struct Query {
    digest: Option<bool>,
}

/// Handles metadata GET requests.
pub async fn get_metadata<C>(
    addr: Address,
    query: Query, // TODO: Use digest
    headers: HeaderMap,
    database: Database,
    peer_handler: PeerHandler<C>,
) -> Result<Response<Body>, GetMetadataError>
where
    C: Clone + Send + Sync,
    C: Connect + 'static,
{
    // Get from database
    let wrapper_opt = database
        .get_metadata(addr.as_body())
        .map_err(GetMetadataError::Database)?;

    // If found in the database
    if let Some(some) = wrapper_opt {
        let raw_auth_wrapper = some.serialized_auth_wrapper;

        // Encode token
        let raw_token = some.token;
        let url_safe_config = base64::Config::new(base64::CharacterSet::UrlSafe, false);
        let token = format!("POP {}", base64::encode_config(raw_token, url_safe_config));

        return Ok(Response::builder()
            .header(AUTHORIZATION, token)
            .body(Body::from(raw_auth_wrapper))
            .unwrap()); // TODO: Headers
    }

    // If MAX_FORWARDS is 0 then don't sample peers
    if headers.get(MAX_FORWARDS) == Some(&HeaderValue::from(0)) {
        return Err(GetMetadataError::NotFound);
    }

    // Sample peers
    let addr_str = addr.encode().unwrap();
    match peer_handler.sample_metadata(&addr_str).await {
        Ok((raw_authwrapper, token)) => Ok(Response::builder()
            .header(AUTHORIZATION, token)
            .body(Body::from(raw_authwrapper))
            .unwrap()),
        _ => Err(GetMetadataError::NotFound),
    }
}

/// Handles metadata PUT requests.
pub async fn put_metadata(
    addr: Address,
    metadata_raw: Bytes,
    token: String,
    db_data: Database,
    token_cache: TokenCache,
) -> Result<Response<Body>, PutMetadataError> {
    // Decode metadata
    let metadata =
        AuthWrapper::decode(metadata_raw.clone()).map_err(PutMetadataError::MetadataDecode)?;

    // Verify signatures
    let pubkey = PublicKey::from_slice(&metadata.pub_key).map_err(PutMetadataError::PublicKey)?;
    if metadata.scheme != 1 {
        // TODO: Support Schnorr
        return Err(PutMetadataError::UnsupportedScheme);
    }
    let signature =
        Signature::from_compact(&metadata.signature).map_err(PutMetadataError::Signature)?;
    let secp = Secp256k1::verification_only();
    let payload_digest = Sha256::digest(&metadata.serialized_payload);
    let msg = Message::from_slice(&payload_digest).map_err(PutMetadataError::Message)?;
    secp.verify(&msg, &signature, &pubkey)
        .map_err(PutMetadataError::InvalidSignature)?;

    // Put to database
    let addr_raw = addr.as_body().to_vec();
    task::spawn_blocking(move || db_data.put_metadata(&addr_raw, &metadata_raw))
        .await
        .unwrap()?;

    // Put token to cache
    token_cache.add_token(addr, token).await;

    // Respond
    Ok(Response::builder().body(Body::empty()).unwrap())
}
