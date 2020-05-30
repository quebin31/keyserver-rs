use std::fmt;

use rocksdb::Error as RocksError;
use secp256k1::Error as SecpError;
use warp::reject::Reject;

use crate::net::IntoResponse;

#[derive(Debug)]
pub enum PutMetadataError {
    Database(RocksError),
    InvalidSignature(SecpError),
    Message(SecpError),
    MetadataDecode(prost::DecodeError),
    PublicKey(SecpError),
    Signature(SecpError),
    UnsupportedScheme,
}

impl From<RocksError> for PutMetadataError {
    fn from(err: RocksError) -> Self {
        Self::Database(err)
    }
}

impl fmt::Display for PutMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
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

impl Reject for PutMetadataError {}

impl IntoResponse for PutMetadataError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Database(_) => 500,
            Self::UnsupportedScheme => 501,
            _ => 400,
        }
    }
}

#[derive(Debug)]
pub enum GetMetadataError {
    NotFound,
    Database(RocksError),
}

impl Reject for GetMetadataError {}

impl From<RocksError> for GetMetadataError {
    fn from(err: RocksError) -> Self {
        Self::Database(err)
    }
}

impl fmt::Display for GetMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            Self::NotFound => "not found",
            Self::Database(err) => return err.fmt(f),
        };
        f.write_str(printable)
    }
}

impl IntoResponse for GetMetadataError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Database(_) => 500,
            _ => 400,
        }
    }
}
