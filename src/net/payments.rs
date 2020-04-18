use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitcoin::{
    consensus::encode::Error as BitcoinError, util::psbt::serialize::Deserialize, Transaction,
};
use bitcoincash_addr::{
    base58::DecodingError as Base58Error, cashaddr::DecodingError as CashAddrError, Address,
};
use cashweb::{
    payments::{
        wallet::{Wallet as WalletGeneric, WalletError},
        PreprocessingError,
    },
    protobuf::bip70::{PaymentAck, PaymentDetails, PaymentRequest},
    token::{schemes::hmac_bearer::HmacTokenScheme, TokenGenerator},
};
use json_rpc::clients::http::HttpConnector;
use prost::Message as _;
use warp::{
    http::{header::AUTHORIZATION, Response},
    hyper::Body,
    reject::Reject,
};

use super::IntoResponse;
use crate::{
    bitcoin::{BitcoinClient, NodeError},
    models::bip70::{Output, Payment},
    PAYMENTS_PATH, SETTINGS,
};

pub type Wallet = WalletGeneric<Vec<u8>, Output>;

#[derive(Debug)]
pub enum PaymentError {
    Preprocess(PreprocessingError),
    Wallet(WalletError),
    MalformedTx(BitcoinError),
    MissingMerchantData,
    Node(NodeError),
}

impl fmt::Display for PaymentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            Self::Preprocess(err) => return err.fmt(f),
            Self::Wallet(err) => return err.fmt(f),
            Self::MalformedTx(err) => return err.fmt(f),
            Self::MissingMerchantData => "missing merchant data",
            Self::Node(err) => return err.fmt(f),
        };
        f.write_str(printable)
    }
}

impl Reject for PaymentError {}

impl IntoResponse for PaymentError {
    fn to_status(&self) -> u16 {
        match self {
            PaymentError::Preprocess(err) => match err {
                PreprocessingError::MissingAcceptHeader => 406,
                PreprocessingError::MissingContentTypeHeader => 415,
                PreprocessingError::PaymentDecode(_) => 400,
            },
            PaymentError::Wallet(err) => match err {
                WalletError::NotFound => 404,
                WalletError::InvalidOutputs => 400,
            },
            PaymentError::MalformedTx(_) => 400,
            PaymentError::MissingMerchantData => 400,
            PaymentError::Node(err) => match err {
                NodeError::Rpc(_) => 400,
                _ => 500,
            },
        }
    }
}

pub async fn process_payment(
    payment: Payment,
    wallet: Wallet,
    bitcoin_client: BitcoinClient<HttpConnector>,
    token_state: Arc<HmacTokenScheme>,
) -> Result<Response<Body>, PaymentError> {
    let txs_res: Result<Vec<Transaction>, BitcoinError> = payment
        .transactions
        .iter()
        .map(|raw_tx| Transaction::deserialize(raw_tx))
        .collect();
    let txs = txs_res.map_err(PaymentError::MalformedTx)?;
    let outputs: Vec<Output> = txs
        .into_iter()
        .map(move |tx| tx.output)
        .flatten()
        .map(|output| Output {
            amount: Some(output.value),
            script: output.script_pubkey.to_bytes(),
        })
        .collect();

    let pubkey_hash = payment
        .merchant_data
        .as_ref()
        .ok_or(PaymentError::MissingMerchantData)?;

    wallet
        .recv_outputs(pubkey_hash, &outputs)
        .map_err(PaymentError::Wallet)?;

    for tx in &payment.transactions {
        bitcoin_client
            .send_tx(tx)
            .await
            .map_err(PaymentError::Node)?;
    }

    // Construct token
    let token = token_state.construct_token(pubkey_hash).unwrap(); // This is safe

    // Create PaymentAck
    let memo = Some(SETTINGS.payments.memo.clone());
    let payment_ack = PaymentAck { payment, memo };

    // Encode payment ack
    let mut raw_ack = Vec::with_capacity(payment_ack.encoded_len());
    payment_ack.encode(&mut raw_ack).unwrap();

    Ok(Response::builder()
        .header(AUTHORIZATION, token)
        .body(Body::from(raw_ack))
        .unwrap())
}

#[derive(Debug)]
pub enum PaymentRequestError {
    Address(CashAddrError, Base58Error),
    Node(NodeError),
    MismatchedNetwork,
}

impl fmt::Display for PaymentRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentRequestError::Address(cash_err, base58_err) => {
                f.write_str(&format!("{}, {}", cash_err, base58_err))
            }
            PaymentRequestError::Node(err) => err.fmt(f),
            PaymentRequestError::MismatchedNetwork => f.write_str("mismatched network"),
        }
    }
}

pub async fn generate_payment_request(
    addr: Address,
    wallet: Wallet,
    bitcoin_client: BitcoinClient<HttpConnector>,
) -> Result<Response<Body>, PaymentRequestError> {
    let output_addr_str = bitcoin_client
        .get_new_addr()
        .await
        .map_err(PaymentRequestError::Node)?;
    let output_addr = Address::decode(&output_addr_str)
        .map_err(|(cash_err, base58_err)| PaymentRequestError::Address(cash_err, base58_err))?;

    // Generate output
    let p2pkh_script_pre: [u8; 3] = [118, 169, 20];
    let p2pkh_script_post: [u8; 2] = [136, 172];
    let script = [
        &p2pkh_script_pre[..],
        output_addr.as_body(),
        &p2pkh_script_post[..],
    ]
    .concat();
    let output = Output {
        amount: Some(SETTINGS.payments.token_fee),
        script,
    };
    let cleanup = wallet.add_outputs(addr.as_body().to_vec(), vec![output.clone()]);
    tokio::spawn(cleanup);

    // Valid interval
    let current_time = SystemTime::now();
    let expiry_time = current_time + Duration::from_millis(SETTINGS.payments.timeout);

    let payment_details = PaymentDetails {
        network: Some(SETTINGS.network.to_string()),
        time: current_time.duration_since(UNIX_EPOCH).unwrap().as_secs(),
        expires: Some(expiry_time.duration_since(UNIX_EPOCH).unwrap().as_secs()),
        memo: None,
        merchant_data: Some(addr.into_body()),
        outputs: vec![output],
        payment_url: Some(format!("/{}", PAYMENTS_PATH)),
    };
    let mut serialized_payment_details = Vec::with_capacity(payment_details.encoded_len());
    payment_details
        .encode(&mut serialized_payment_details)
        .unwrap();

    // Generate payment invoice
    // TODO: Signing
    let pki_type = Some("none".to_string());
    let payment_invoice = PaymentRequest {
        pki_type,
        pki_data: None,
        payment_details_version: Some(1),
        serialized_payment_details,
        signature: None,
    };
    let mut payment_invoice_raw = Vec::with_capacity(payment_invoice.encoded_len());
    payment_invoice.encode(&mut payment_invoice_raw).unwrap();

    Ok(Response::builder()
        .status(402)
        .body(Body::from(payment_invoice_raw))
        .unwrap())
}
