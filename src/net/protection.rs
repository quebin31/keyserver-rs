use std::{fmt, sync::Arc};

use bitcoincash_addr::Address;
use cashweb::token::{extract_pop, schemes::hmac_bearer::*, TokenValidator};
use http::header::HeaderMap;
use json_rpc::clients::http::HttpConnector;
use warp::{http::Response, hyper::Body, reject::Reject};

use crate::{
    bitcoin::BitcoinClient,
    net::payments::{generate_payment_request, Wallet},
};

#[derive(Debug)]
pub enum ProtectionError {
    MissingToken(Address, Wallet, BitcoinClient<HttpConnector>),
    Validation(ValidationError),
}

impl fmt::Display for ProtectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken(_, _, _) => f.write_str("missing token"),
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
        ProtectionError::MissingToken(addr, wallet, bitcoin_client) => {
            // TODO: Remove clones here
            match generate_payment_request(addr.clone(), wallet.clone(), bitcoin_client.clone())
                .await
            {
                Ok(ok) => ok,
                Err(err) => Response::builder()
                    .status(400)
                    .body(Body::from(err.to_string()))
                    .unwrap(),
            }
        }
    }
}

impl Reject for ProtectionError {}

pub async fn pop_protection(
    addr: Address,
    header_map: HeaderMap,
    token_scheme: Arc<HmacTokenScheme>,
    wallet: Wallet,
    bitcoin_client: BitcoinClient<HttpConnector>,
) -> Result<Address, ProtectionError> {
    match extract_pop(&header_map) {
        Some(pop_token) => {
            token_scheme
                .validate_token(&addr.as_body().to_vec(), pop_token)
                .await
                .map_err(ProtectionError::Validation)?;
            Ok(addr)
        }
        None => Err(ProtectionError::MissingToken(addr, wallet, bitcoin_client)),
    }
}
