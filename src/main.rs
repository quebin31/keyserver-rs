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
    bitcoin_client::BitcoinClient, payments::preprocess_payment,
    token::schemes::chain_commitment::ChainCommitmentScheme,
};
use futures::prelude::*;
use hyper::client::HttpConnector;
use lazy_static::lazy_static;
use log::info;
#[cfg(feature = "monitoring")]
use prometheus::{Encoder, TextEncoder};
use warp::{
    http::{header, Method},
    Filter,
};

use db::Database;
use net::{payments, protection};
use peering::{PeerHandler, TokenCache};
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

    // Fetch peers from settings
    let peers_settings: Vec<_> = SETTINGS.peering.peers.clone();

    // Retrieve saved peers from database
    let peers_opt = db.get_peers().unwrap(); // Unrecoverable
    let peers_db: std::collections::HashSet<String> = peers_opt
        .unwrap_or_default()
        .peers
        .into_iter()
        .map(|peer| peer.url)
        .collect();

    // Combine collected peers
    let mut peers = peers_db;
    for peer in peers_settings.into_iter() {
        peers.insert(peer);
    }

    // Setup peer connector
    let mut connector = HttpConnector::new();
    connector.set_keepalive(Some(Duration::from_secs(SETTINGS.peering.keep_alive)));
    connector.set_connect_timeout(Some(Duration::from_secs(SETTINGS.peering.timeout)));

    // Setup peer state
    let mut peer_handler = PeerHandler::new(peers, connector);
    let new_peers = peer_handler.traverse().await;
    peer_handler.set_peers(new_peers).await;

    // Persist peers
    if let Err(err) = peer_handler.persist(&db).await {
        log::error!("failed to persist new peer state; {}", err);
    }

    // Token cache
    let token_cache = TokenCache::default();

    // Setup ZMQ stream
    let mut subscriber = async_zmq::subscribe(&SETTINGS.bitcoin_rpc.zmq_address)
        .unwrap()
        .connect()
        .unwrap();
    subscriber.set_subscribe("hashblock").unwrap(); // Unrecoverable

    // Start broadcast heartbeat
    let token_cache_inner = token_cache.clone();
    let peer_handler_inner = peer_handler.clone();
    let db_inner = db.clone();
    let broadcast_heartbeat = || async move {
        while let Some(val) = subscriber.next().await {
            if let Ok(inner) = val {
                if let Some(block) = inner.get(1) {
                    info!("found block {}", hex::encode(block.as_ref()));
                    token_cache_inner
                        .broadcast_block(&peer_handler_inner, &db_inner)
                        .await;
                }
            }
        }
    };
    tokio::spawn(broadcast_heartbeat());

    // Peer state
    let peer_handler = warp::any().map(move || peer_handler.clone());

    // Database state
    let db_state = warp::any().map(move || db.clone());

    // Initialize bitcoin client
    let bitcoin_client = BitcoinClient::new(
        SETTINGS.bitcoin_rpc.address.clone(),
        SETTINGS.bitcoin_rpc.username.clone(),
        SETTINGS.bitcoin_rpc.password.clone(),
    );

    // Address string converter
    let addr_base = warp::path::param().and_then(|addr_str: String| async move {
        net::address_decode(&addr_str).map_err(warp::reject::custom)
    });

    // Token generator
    let token_scheme = Arc::new(ChainCommitmentScheme::from_client(bitcoin_client.clone()));
    let token_scheme_state = warp::any().map(move || token_scheme.clone());

    // Token cache state
    let token_cache_state = warp::any().map(move || token_cache.clone());

    // Bitcoin client state
    let bitcoin_client_state = warp::any().map(move || bitcoin_client.clone());

    // Protection
    let addr_protected = addr_base
        .clone()
        .and(warp::body::content_length_limit(
            SETTINGS.limits.metadata_size,
        ))
        .and(warp::body::bytes())
        .and(warp::header::headers_cloned())
        .and(token_scheme_state.clone())
        .and_then(move |addr, body, headers, token_scheme| {
            protection::pop_protection(addr, body, headers, token_scheme)
                .map_err(warp::reject::custom)
        })
        .untuple_one();

    // Metadata handlers
    let metadata_get = warp::path(METADATA_PATH)
        .and(addr_base)
        .and(warp::get())
        .and(warp::header::headers_cloned())
        .and(db_state.clone())
        .and(peer_handler.clone())
        .and_then(move |addr, headers, db, peer_handler| {
            net::get_metadata(addr, headers, db, peer_handler).map_err(warp::reject::custom)
        });
    let metadata_put = warp::path(METADATA_PATH)
        .and(addr_protected)
        .and(warp::put())
        .and(warp::body::content_length_limit(
            SETTINGS.limits.metadata_size,
        ))
        .and(db_state.clone())
        .and(token_cache_state)
        .and_then(
            move |addr, auth_wrapper_raw, auth_wrapper, raw_token, db, token_cache| {
                net::put_metadata(
                    addr,
                    auth_wrapper_raw,
                    auth_wrapper,
                    raw_token,
                    db,
                    token_cache,
                )
                .map_err(warp::reject::custom)
            },
        );

    // Peer handler
    let peers_get = warp::path(PEERS_PATH)
        .and(warp::get())
        .and(peer_handler)
        .and_then(move |peer_handler| net::get_peers(peer_handler).map_err(warp::reject::custom));

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
        .and(bitcoin_client_state.clone())
        .and_then(move |payment, bitcoin_client| async move {
            net::process_payment(payment, bitcoin_client)
                .await
                .map_err(warp::reject::custom)
        });

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
