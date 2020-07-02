pub mod errors;

use std::fmt;

use bitcoincash_addr::Address;
use bytes::Bytes;
use http::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Request,
};
use prost::Message as _;
use tokio::task;
use tower_service::Service;
use warp::{http::Response, hyper::Body};

use super::{HEADER_VALUE_FALSE, SAMPLING};
use crate::{
    db::Database,
    models::{database::DatabaseWrapper, wrapper::AuthWrapper},
    peering::{PeerHandler, TokenCache},
    SETTINGS,
};
pub use errors::*;

/// Handles metadata GET requests.
pub async fn get_metadata<S>(
    addr: Address,
    headers: HeaderMap,
    database: Database,
    peer_handler: PeerHandler<S>,
) -> Result<Response<Body>, GetMetadataError>
where
    S: Service<Request<Body>, Response = Response<Body>>,
    S: Send + Clone + 'static,
    S::Future: Send,
    S::Error: fmt::Debug + Send,
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
    match peer_handler
        .get_keyserver_manager()
        .uniform_sample_metadata(&addr_str, SETTINGS.peering.pull_fan_size)
        .await
    {
        Ok(sample_response) => {
            let metadata_package = sample_response.response;
            let token = metadata_package.token;
            let raw_auth_wrapper = metadata_package.raw_auth_wrapper;
            Ok(Response::builder()
                .header(AUTHORIZATION, token)
                .body(Body::from(raw_auth_wrapper))
                .unwrap())
        }
        _ => Err(GetMetadataError::NotFound),
    }
}

/// Handles metadata PUT requests.
pub async fn put_metadata(
    addr: Address,
    auth_wrapper_raw: Bytes,
    auth_wrapper: AuthWrapper,
    token_raw: Vec<u8>,
    db_data: Database,
    token_cache: TokenCache,
) -> Result<Response<Body>, PutMetadataError> {
    // Verify signatures
    auth_wrapper
        .parse()
        .map_err(PutMetadataError::InvalidAuthWrapper)?
        .verify()
        .map_err(PutMetadataError::VerifyAuthWrapper)?;

    // Wrap with database
    let database_wrapper = DatabaseWrapper {
        serialized_auth_wrapper: auth_wrapper_raw.to_vec(),
        token: token_raw,
    };
    let mut raw_database_wrapper = Vec::with_capacity(database_wrapper.encoded_len());
    database_wrapper.encode(&mut raw_database_wrapper).unwrap(); // This is safe

    // Put to database
    let addr_raw = addr.as_body().to_vec();
    task::spawn_blocking(move || db_data.put_metadata(&addr_raw, &raw_database_wrapper))
        .await
        .unwrap()?;

    // Put token to cache
    token_cache.add_token(addr).await;

    // Respond
    Ok(Response::builder().body(Body::empty()).unwrap())
}
