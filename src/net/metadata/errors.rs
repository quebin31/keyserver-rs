use rocksdb::Error as RocksError;
use thiserror::Error;
use warp::reject::Reject;

use crate::{
    models::wrapper::{ParseError, VerifyError},
    net::IntoResponse,
};

#[derive(Debug, Error)]
pub enum PutMetadataError {
    #[error("failed to write to database: {0}")]
    Database(RocksError),
    #[error("failed to decode authorization wrapper: {0}")]
    MetadataDecode(prost::DecodeError),
    #[error("failed to verify authorization wrapper: {0}")]
    InvalidAuthWrapper(ParseError),
    #[error("failed to parse authorization wrapper: {0}")]
    VerifyAuthWrapper(VerifyError),
}

impl From<RocksError> for PutMetadataError {
    fn from(err: RocksError) -> Self {
        Self::Database(err)
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

#[derive(Debug, Error)]
pub enum GetMetadataError {
    #[error("not found")]
    NotFound,
    #[error("failed to read from database: {0}")]
    Database(RocksError),
}

impl Reject for GetMetadataError {}

impl From<RocksError> for GetMetadataError {
    fn from(err: RocksError) -> Self {
        Self::Database(err)
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
