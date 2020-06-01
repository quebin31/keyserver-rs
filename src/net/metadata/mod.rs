pub mod errors;

use bitcoincash_addr::Address;
use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use hyper::client::connect::Connect;
use prost::Message as _;
use tokio::task;
use warp::{http::Response, hyper::Body};

use crate::{
    db::Database,
    models::{database::DatabaseWrapper, wrapper::AuthWrapper},
    peering::PeerHandler,
    peering::TokenCache,
};
use super::{HEADER_VALUE_FALSE, SAMPLING};
pub use errors::*;

/// Handles metadata GET requests.
pub async fn get_metadata<C>(
    addr: Address,
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
    if headers.get(SAMPLING) == Some(&HeaderValue::from_static(HEADER_VALUE_FALSE)) {
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
    token_str: String,
    token_raw: Vec<u8>,
    db_data: Database,
    token_cache: TokenCache,
) -> Result<Response<Body>, PutMetadataError> {
    // Decode metadata
    let metadata =
        AuthWrapper::decode(metadata_raw.clone()).map_err(PutMetadataError::MetadataDecode)?;

    // Wrap with database
    let database_wrapper = DatabaseWrapper {
        serialized_auth_wrapper: metadata_raw.to_vec(),
        token: token_raw,
    };
    let mut raw_database_wrapper = Vec::with_capacity(database_wrapper.encoded_len());
    database_wrapper.encode(&mut raw_database_wrapper).unwrap(); // This is safe

    // Verify signatures
    metadata
        .validate()
        .map_err(PutMetadataError::InvalidAuthWrapper)?;

    // Put to database
    let addr_raw = addr.as_body().to_vec();
    task::spawn_blocking(move || db_data.put_metadata(&addr_raw, &raw_database_wrapper))
        .await
        .unwrap()?;

    // Put token to cache
    token_cache.add_token(addr, token_str).await;

    // Respond
    Ok(Response::builder().body(Body::empty()).unwrap())
}
