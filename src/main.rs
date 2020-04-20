#[macro_use]
extern crate clap;
#[macro_use]
extern crate serde;

pub mod bitcoin;
pub mod db;
pub mod models;
pub mod net;
pub mod peering;
pub mod settings;

#[cfg(feature = "monitoring")]
pub mod monitoring;

use std::{env, sync::Arc, time::Duration};

use cashweb::{
    payments::{preprocess_payment, wallet::Wallet},
    token::schemes::hmac_bearer::HmacTokenScheme,
};
use futures::prelude::*;
use hyper::client::HttpConnector;
use lazy_static::lazy_static;
#[cfg(feature = "monitoring")]
use prometheus::{Encoder, TextEncoder};
use tokio::sync::RwLock;
use warp::{
    http::{header, Method},
    Filter,
};

use crate::bitcoin::BitcoinClient;
use db::Database;
use net::{payments, protection};
use peering::PeerState;
use settings::Settings;

const METADATA_PATH: &str = "keys";
const PEERS_PATH: &str = "peers";
pub const PAYMENTS_PATH: &str = "payments";

lazy_static! {
    // Static settings
    pub static ref SETTINGS: Settings = Settings::new().expect("couldn't load config");
}

#[tokio::main]
async fn main() {
    if env::var_os("RUST_LOG").is_none() {
        env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    // Initialize database
    let db = Database::try_new(&SETTINGS.db_path).expect("failed to open database");

    // Initialize peering
    let peers_opt = db.get_peers().unwrap(); // Unrecoverable
    let peers = peers_opt.unwrap_or_default();
    let mut connector = HttpConnector::new();
    connector.set_keepalive(Some(Duration::from_secs(30)));
    connector.set_connect_timeout(Some(Duration::from_secs(60)));
    let mut peer_state = PeerState::new(peers, connector);
    let new_peers = peer_state.traverse().await;
    peer_state.set_peers(new_peers);
    if let Err(err) = peer_state.persist(&db) {
        log::error!("failed to persist new peer state; {}", err);
    }

    // Peer state
    let shared_peering = Arc::new(RwLock::new(peer_state));
    let peer_state = warp::any().map(move || shared_peering.clone());

    // Database state
    let db_state = warp::any().map(move || db.clone());

    // Wallet state
    let wallet = Wallet::new(Duration::from_millis(SETTINGS.payments.timeout));
    let wallet_state = warp::any().map(move || wallet.clone());

    // Bitcoin client state
    let bitcoin_client = BitcoinClient::new(
        SETTINGS.bitcoin_rpc.address.clone(),
        SETTINGS.bitcoin_rpc.username.clone(),
        SETTINGS.bitcoin_rpc.password.clone(),
    );
    let bitcoin_client_state = warp::any().map(move || bitcoin_client.clone());

    // Address string converter
    let addr_base = warp::path::param().and_then(|addr_str: String| async move {
        net::address_decode(&addr_str).map_err(warp::reject::custom)
    });

    // Token generator
    let key =
        hex::decode(&SETTINGS.payments.hmac_secret).expect("unable to interpret hmac key as hex");
    let token_scheme = Arc::new(HmacTokenScheme::new(&key));
    let token_scheme_state = warp::any().map(move || token_scheme.clone());

    // Protection
    let addr_protected = addr_base
        .clone()
        .and(warp::header::headers_cloned())
        .and(token_scheme_state.clone())
        .and(wallet_state.clone())
        .and(bitcoin_client_state.clone())
        .and_then(move |addr, headers, token_scheme, wallet, bitcoin| {
            protection::pop_protection(addr, headers, token_scheme, wallet, bitcoin)
                .map_err(warp::reject::custom)
        });

    // Metadata handlers
    let metadata_get = warp::path(METADATA_PATH)
        .and(addr_base)
        .and(warp::get())
        .and(warp::query())
        .and(db_state.clone())
        .and_then(move |addr, query, db| {
            net::get_metadata(addr, query, db).map_err(warp::reject::custom)
        });
    let metadata_put = warp::path(METADATA_PATH)
        .and(addr_protected)
        .and(warp::put())
        .and(warp::body::content_length_limit(
            SETTINGS.limits.metadata_size,
        ))
        .and(warp::body::bytes())
        .and(db_state.clone())
        .and_then(move |addr, body, db| {
            net::put_metadata(addr, body, db).map_err(warp::reject::custom)
        });

    // Peer handler
    let peers_get = warp::path(PEERS_PATH)
        .and(warp::get())
        .and(peer_state)
        .and_then(move |peer_state| net::get_peers(peer_state).map_err(warp::reject::custom));

    // Payment handler
    let payments = warp::path(PAYMENTS_PATH)
        .and(warp::post())
        .and(warp::header::headers_cloned())
        .and(warp::body::content_length_limit(
            SETTINGS.limits.metadata_size,
        ))
        .and(warp::body::bytes())
        .and_then(move |headers, body| {
            preprocess_payment(headers, body)
                .map_err(payments::PaymentError::Preprocess)
                .map_err(warp::reject::custom)
        })
        .and(wallet_state.clone())
        .and(bitcoin_client_state.clone())
        .and(token_scheme_state)
        .and_then(
            move |payment, wallet, bitcoin_client, token_state| async move {
                net::process_payment(payment, wallet, bitcoin_client, token_state)
                    .await
                    .map_err(warp::reject::custom)
            },
        );

    // Root handler
    let root = warp::path::end()
        .and(warp::get())
        .and(warp::fs::file("./static/index.html"));

    // CORs
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec![Method::GET, Method::PUT, Method::POST, Method::DELETE])
        .allow_headers(vec![header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers(vec![
            header::AUTHORIZATION,
            header::ACCEPT,
            header::LOCATION,
        ])
        .build();

    // If monitoring is enabled
    #[cfg(feature = "monitoring")]
    {
        // Init Prometheus server
        let prometheus_server = warp::path("metrics").map(monitoring::export);
        let prometheus_task = warp::serve(prometheus_server).run(SETTINGS.bind_prom);

        // Init REST API
        let rest_api = root
            .or(payments)
            .or(metadata_get)
            .or(metadata_put)
            .or(peers_get)
            .recover(net::handle_rejection)
            .with(cors)
            .with(warp::log("keyserver"))
            .with(warp::log::custom(monitoring::measure));
        let rest_api_task = warp::serve(rest_api).run(SETTINGS.bind);

        // Spawn servers
        tokio::spawn(prometheus_task);
        tokio::spawn(rest_api_task).await.unwrap(); // Unrecoverable
    }

    // If monitoring is disabled
    #[cfg(not(feature = "monitoring"))]
    {
        // Init REST API
        let rest_api = root
            .or(payments)
            .or(metadata_get)
            .or(metadata_put)
            .or(peers_get)
            .recover(net::handle_rejection)
            .with(cors)
            .with(warp::log("keyserver"));
        let rest_api_task = warp::serve(rest_api).run(SETTINGS.bind);
        tokio::spawn(rest_api_task).await.unwrap(); // Unrecoverable
    }
}
