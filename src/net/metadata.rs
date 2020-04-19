use std::fmt;

use bitcoincash_addr::Address;
use bytes::Bytes;
use prost::Message as _;
use rocksdb::Error as RocksError;
use secp256k1::{key::PublicKey, Error as SecpError, Message, Secp256k1, Signature};
use sha2::{Digest, Sha256};
use tokio::task;
use warp::{http::Response, hyper::Body, reject::Reject};

use super::IntoResponse;
use crate::{db::Database, models::wrapper::AuthWrapper};

#[derive(Debug)]
pub enum MetadataError {
    NotFound,
    Database(RocksError),
    InvalidSignature(SecpError),
    Message(SecpError),
    MetadataDecode(prost::DecodeError),
    PublicKey(SecpError),
    Signature(SecpError),
    UnsupportedScheme,
}

#[derive(Debug, Deserialize)]
pub struct Query {
    digest: Option<bool>,
}

impl From<RocksError> for MetadataError {
    fn from(err: RocksError) -> Self {
        MetadataError::Database(err)
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            Self::NotFound => "not found",
            Self::Database(err) => return err.fmt(f),
            Self::InvalidSignature(err) => return err.fmt(f),
            Self::Message(err) => return err.fmt(f),
            Self::MetadataDecode(err) => return err.fmt(f),
            Self::PublicKey(err) => return err.fmt(f),
            Self::Signature(err) => return err.fmt(f),
            Self::UnsupportedScheme => "unsupported signature scheme",
        };
        f.write_str(printable)
    }
}

impl Reject for MetadataError {}

impl IntoResponse for MetadataError {
    fn to_status(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::Database(_) => 500,
            Self::UnsupportedScheme => 501,
            _ => 400,
        }
    }
}

pub async fn get_metadata(
    addr: Address,
    query: Query,
    database: Database,
) -> Result<Response<Body>, MetadataError> {
    // Get metadata
    let metadata = task::spawn_blocking(move || database.get_metadata(addr.as_body()))
        .await
        .unwrap()?
        .ok_or(MetadataError::NotFound)?;

    // Serialize messages
    let mut raw_metadata = Vec::with_capacity(metadata.encoded_len());
    metadata.encode(&mut raw_metadata).unwrap();

    // Respond
    match query.digest {
        Some(true) => {
            let digest = Sha256::digest(&raw_metadata).to_vec();
            Ok(Response::builder().body(Body::from(digest)).unwrap()) // TODO: Headers
        }
        _ => {
            Ok(Response::builder().body(Body::from(raw_metadata)).unwrap()) // TODO: Headers
        }
    }
}

pub async fn put_metadata(
    addr: Address,
    metadata_raw: Bytes,
    db_data: Database,
) -> Result<Response<Body>, MetadataError> {
    // Decode metadata
    let metadata =
        AuthWrapper::decode(metadata_raw.clone()).map_err(MetadataError::MetadataDecode)?;

    // Verify signatures
    let pubkey = PublicKey::from_slice(&metadata.pub_key).map_err(MetadataError::PublicKey)?;
    if metadata.scheme != 1 {
        // TODO: Support Schnorr
        return Err(MetadataError::UnsupportedScheme);
    }
    let signature =
        Signature::from_compact(&metadata.signature).map_err(MetadataError::Signature)?;
    let secp = Secp256k1::verification_only();
    let payload_digest = Sha256::digest(&metadata.serialized_payload);
    let msg = Message::from_slice(&payload_digest).map_err(MetadataError::Message)?;
    secp.verify(&msg, &signature, &pubkey)
        .map_err(MetadataError::InvalidSignature)?;

    // Put to database
    task::spawn_blocking(move || db_data.put_metadata(addr.as_body(), &metadata_raw))
        .await
        .unwrap()?;

    // Respond
    Ok(Response::builder().body(Body::empty()).unwrap())
}
