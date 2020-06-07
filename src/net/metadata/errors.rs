use std::fmt;

use rocksdb::Error as RocksError;
use warp::reject::Reject;

use crate::{models::wrapper::ValidationError, net::IntoResponse};

#[derive(Debug)]
pub enum PutMetadataError {
    Database(RocksError),
    MetadataDecode(prost::DecodeError),
    InvalidAuthWrapper(ValidationError),
}

impl From<RocksError> for PutMetadataError {
    fn from(err: RocksError) -> Self {
        Self::Database(err)
    }
}

impl fmt::Display for PutMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => err.fmt(f),
            Self::MetadataDecode(err) => err.fmt(f),
            Self::InvalidAuthWrapper(err) => err.fmt(f),
        }
    }
}

impl Reject for PutMetadataError {}

impl IntoResponse for PutMetadataError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Database(_) => 500,
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
            Self::NotFound => 404,
            Self::Database(_) => 500,
        }
    }
}
