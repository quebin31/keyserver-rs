use std::fmt;

use bitcoin::network::constants;
pub use json_rpc::clients::http::HttpConnector;
use json_rpc::prelude::*;

use serde_json::Value;

#[derive(Copy, Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet = 0,
    Testnet = 1,
    Regtest = 2,
}

impl From<bitcoincash_addr::Network> for Network {
    fn from(network: bitcoincash_addr::Network) -> Network {
        match network {
            bitcoincash_addr::Network::Main => Network::Mainnet,
            bitcoincash_addr::Network::Test => Network::Testnet,
            bitcoincash_addr::Network::Regtest => Network::Regtest,
        }
    }
}

impl Into<bitcoincash_addr::Network> for Network {
    fn into(self) -> bitcoincash_addr::Network {
        match self {
            Network::Mainnet => bitcoincash_addr::Network::Main,
            Network::Testnet => bitcoincash_addr::Network::Test,
            Network::Regtest => bitcoincash_addr::Network::Regtest,
        }
    }
}

impl Into<bitcoin::network::constants::Network> for Network {
    fn into(self) -> bitcoin::network::constants::Network {
        match self {
            Network::Mainnet => constants::Network::Bitcoin,
            Network::Testnet => constants::Network::Testnet,
            Network::Regtest => constants::Network::Regtest,
        }
    }
}

impl ToString for Network {
    fn to_string(&self) -> String {
        match self {
            Network::Mainnet => "mainnet".to_string(),
            Network::Testnet => "testnet".to_string(),
            Network::Regtest => "regtest".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BitcoinClient<C>(HttpClient<C>);

impl BitcoinClient<HttpConnector> {
    pub fn new(endpoint: String, username: String, password: String) -> Self {
        BitcoinClient(HttpClient::new(endpoint, Some(username), Some(password)))
    }
}

impl<C> std::ops::Deref for BitcoinClient<C> {
    type Target = HttpClient<C>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub enum NodeError {
    Http(HttpError),
    Rpc(RpcError),
    Json(JsonError),
    EmptyResponse,
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(err) => err.fmt(f),
            Self::Json(err) => err.fmt(f),
            Self::Rpc(err) => f.write_str(&format!("{:#?}", err)),
            Self::EmptyResponse => f.write_str("empty response"),
        }
    }
}

impl<C> BitcoinClient<C>
where
    C: Connect + Clone + Send + Sync + 'static,
{
    pub async fn get_new_addr(&self) -> Result<String, NodeError> {
        let request = self
            .build_request()
            .method("getnewaddress")
            .finish()
            .unwrap();
        let response = self.send(request).await.map_err(NodeError::Http)?;
        if response.is_error() {
            return Err(NodeError::Rpc(response.error().unwrap()));
        }
        response
            .into_result()
            .ok_or(NodeError::EmptyResponse)?
            .map_err(NodeError::Json)
    }

    pub async fn send_tx(&self, raw_tx: &[u8]) -> Result<String, NodeError> {
        let request = self
            .build_request()
            .method("sendrawtransaction")
            .params(vec![Value::String(hex::encode(raw_tx))])
            .finish()
            .unwrap();
        let response = self.send(request).await.map_err(NodeError::Http)?;
        if response.is_error() {
            let err = response.error().unwrap();
            return Err(NodeError::Rpc(err));
        }
        response
            .into_result()
            .ok_or(NodeError::EmptyResponse)?
            .map_err(NodeError::Json)
    }
}
