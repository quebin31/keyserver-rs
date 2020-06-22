use std::{collections::HashSet, str::FromStr};

use bytes::Bytes;
use http::{
    header::{ToStrError, AUTHORIZATION},
    uri::InvalidUri,
};
use hyper::{
    body::{aggregate, to_bytes},
    client::connect::Connect,
    Body, Client, Error as HyperError, Request, Uri,
};
use prost::{DecodeError, Message as _};

use crate::{
    models::{
        keyserver::Peers,
        wrapper::{AuthWrapper, ParseError, VerifyError},
    },
    net::{HEADER_VALUE_FALSE, SAMPLING},
    METADATA_PATH,
};

#[derive(Clone)]
pub struct PeeringClient<C>(Client<C, Body>);

#[derive(Debug)]
pub enum PeerError {
    Hyper(HyperError),
    AuthWrapperDecode(prost::DecodeError),
    InvalidAuthWrapper(ParseError),
    InvalidHeader(ToStrError),
    PeerDecode(DecodeError),
    Uri(InvalidUri),
    VerifyAuthWrapper(VerifyError),
    MissingToken,
    NotFound,
}

impl From<HyperError> for PeerError {
    fn from(err: HyperError) -> Self {
        Self::Hyper(err)
    }
}

impl From<InvalidUri> for PeerError {
    fn from(err: InvalidUri) -> Self {
        Self::Uri(err)
    }
}

impl<C> PeeringClient<C>
where
    C: Clone + Send + Sync,
    C: Connect + 'static,
{
    pub fn new(connector: C) -> Self {
        PeeringClient(Client::builder().build::<_, Body>(connector))
    }

    /// Get `Peers` from a peer.
    pub async fn get_peers(&self, url: String) -> Result<Vec<String>, PeerError> {
        let uri = Uri::from_str(&format!("{}/peers", url))?;
        let response = self.0.get(uri).await?;
        let raw = aggregate(response.into_body()).await?;
        let peers = Peers::decode(raw).map_err(PeerError::PeerDecode)?;
        Ok(peers.peers.into_iter().map(|peer| peer.url).collect())
    }

    /// Get multiple `Peers` from a selection of peers.
    pub async fn get_peer_fan(&self, url_set: &HashSet<String>) -> HashSet<String> {
        let fan = url_set.iter().map(|url| self.get_peers(url.clone()));
        let new_urls: HashSet<_> = futures::future::join_all(fan)
            .await
            .into_iter()
            .filter_map(|urls| urls.ok())
            .flatten()
            .collect();
        new_urls
    }

    /// Get an `AuthWrapper` from a peer.
    pub async fn get_metadata(&self, url: &str, addr: &str) -> Result<(Bytes, String), PeerError> {
        let url = format!("{}/{}/{}", url, METADATA_PATH, addr);
        let uri = Uri::from_str(&url)?;
        let request = Request::get(uri)
            .header(SAMPLING, HEADER_VALUE_FALSE)
            .body(Body::empty())
            .unwrap(); // This is safe
        let response = self.0.request(request).await?;
        let token = response
            .headers()
            .get(AUTHORIZATION)
            .ok_or(PeerError::MissingToken)?
            .to_str()
            .map_err(PeerError::InvalidHeader)?
            .to_string();

        let raw_auth_wrapper = to_bytes(response.into_body()).await?;
        let auth_wrapper =
            AuthWrapper::decode(raw_auth_wrapper.clone()).map_err(PeerError::AuthWrapperDecode)?;
        auth_wrapper
            .parse()
            .map_err(PeerError::InvalidAuthWrapper)?
            .verify()
            .map_err(PeerError::VerifyAuthWrapper)?;
        Ok((raw_auth_wrapper, token))
    }

    /// Get multiple `AuthWrapper` from a selection of peers.
    pub async fn get_metadata_fan(&self, addr: &str, url_set: &[String]) -> Vec<(Bytes, String)> {
        let fan = url_set.iter().map(|url| self.get_metadata(url, addr));
        futures::future::join_all(fan)
            .await
            .into_iter()
            .filter_map(|urls| urls.ok())
            .collect()
    }

    /// Put a serialized `AuthWrapper` to a peer.
    pub async fn put_metadata(
        &self,
        url: &str,
        addr: &str,
        raw_auth_wrapper: Bytes,
        token: &str,
    ) -> Result<(), PeerError> {
        let uri = Uri::from_str(&format!("{}/{}/{}", url, METADATA_PATH, addr))?;
        let request = Request::put(uri)
            .header(AUTHORIZATION, token)
            .body(Body::from(raw_auth_wrapper))
            .unwrap(); // This is safe
        self.0.request(request).await?;
        Ok(())
    }
}
