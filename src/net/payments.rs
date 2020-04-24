use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitcoin::{
    consensus::encode::Error as BitcoinError, util::psbt::serialize::Deserialize, Transaction,
};
use bitcoincash_addr::{
    base58::DecodingError as Base58Error,
    cashaddr::{DecodingError as CashAddrError, EncodingError as AddrEncodingError},
    Address,
};
use cashweb::bitcoin_client::{BitcoinClient, HttpConnector, NodeError};
use cashweb::{
    payments::PreprocessingError,
    protobuf::bip70::{PaymentAck, PaymentDetails, PaymentRequest},
    token::schemes::chain_commitment::*,
};
use prost::Message as _;
use sha2::{Digest, Sha256};
use warp::{
    http::{
        header::{AUTHORIZATION, LOCATION},
        Response,
    },
    hyper::Body,
    reject::Reject,
};

use super::{address_decode, IntoResponse};
use crate::{
    models::bip70::{Output, Payment},
    METADATA_PATH, PAYMENTS_PATH, SETTINGS,
};

pub const COMMITMENT_PREIMAGE_SIZE: usize = 20 + 32;
pub const COMMITMENT_SIZE: usize = 32;
pub const OP_RETURN: u8 = 106;

#[derive(Debug)]
pub enum PaymentError {
    Preprocess(PreprocessingError),
    MissingCommitment,
    MalformedTx(BitcoinError),
    MissingMerchantData,
    Node(NodeError),
    IncorrectLengthPreimage,
    Address(AddrEncodingError),
}

impl fmt::Display for PaymentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            Self::Address(err) => return err.fmt(f),
            Self::Preprocess(err) => return err.fmt(f),
            Self::MissingCommitment => "missing commitment",
            Self::MalformedTx(err) => return err.fmt(f),
            Self::MissingMerchantData => "missing merchant data",
            Self::Node(err) => return err.fmt(f),
            Self::IncorrectLengthPreimage => "incorrect length preimage",
        };
        f.write_str(printable)
    }
}

impl Reject for PaymentError {}

impl IntoResponse for PaymentError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Address(_) => 400,
            Self::IncorrectLengthPreimage => 400,
            Self::Preprocess(err) => match err {
                PreprocessingError::MissingAcceptHeader => 406,
                PreprocessingError::MissingContentTypeHeader => 415,
                PreprocessingError::PaymentDecode(_) => 400,
            },
            Self::MalformedTx(_) => 400,
            Self::MissingMerchantData => 400,
            Self::MissingCommitment => 400,
            Self::Node(err) => match err {
                NodeError::Rpc(_) => 400,
                _ => 500,
            },
        }
    }
}

pub async fn process_payment(
    payment: Payment,
    bitcoin_client: BitcoinClient<HttpConnector>,
) -> Result<Response<Body>, PaymentError> {
    // Deserialize transactions
    let txs_res: Result<Vec<Transaction>, BitcoinError> = payment
        .transactions
        .iter()
        .map(|raw_tx| Transaction::deserialize(raw_tx))
        .collect();
    let txs = txs_res.map_err(PaymentError::MalformedTx)?;

    // Find commitment output
    let commitment_preimage = payment
        .merchant_data
        .as_ref()
        .ok_or(PaymentError::MissingMerchantData)?;

    if commitment_preimage.len() != COMMITMENT_PREIMAGE_SIZE {
        return Err(PaymentError::IncorrectLengthPreimage);
    }

    // Get address
    let pub_key_hash = &commitment_preimage[..20];
    let address = Address {
        body: pub_key_hash.to_vec(),
        ..Default::default()
    };
    let addr_str = address.encode().map_err(PaymentError::Address)?;

    // Extract metadata
    let address_metadata_hash = &commitment_preimage[20..COMMITMENT_PREIMAGE_SIZE];

    let expected_commitment = construct_commitment(pub_key_hash, address_metadata_hash);

    log::info!("expected: {}", hex::encode(&expected_commitment));
    let (tx_id, vout) = txs
        .iter()
        .find_map(|tx| {
            tx.output
                .iter()
                .enumerate()
                .find_map(|(vout, output)| {
                    let raw_script = output.script_pubkey.to_bytes();
                    if raw_script.len() == 2 + COMMITMENT_SIZE
                        && raw_script[0] == OP_RETURN
                        && raw_script[1] == COMMITMENT_SIZE as u8
                        && raw_script[2..34] == expected_commitment[..]
                    {
                        Some(vout)
                    } else {
                        None
                    }
                })
                .map(|vout| {
                    let mut tx_id = tx.txid().to_vec();
                    tx_id.reverse();
                    (tx_id, vout)
                })
        })
        .ok_or(PaymentError::MissingCommitment)?;

    // Broadcast transactions
    for tx in &payment.transactions {
        bitcoin_client
            .send_tx(tx)
            .await
            .map_err(PaymentError::Node)?;
    }

    // Construct token
    let token = format!("POP {}", construct_token(&tx_id, vout as u32));

    // Create PaymentAck
    let memo = Some(SETTINGS.payments.memo.clone());
    let payment_ack = PaymentAck { payment, memo };

    // Encode payment ack
    let mut raw_ack = Vec::with_capacity(payment_ack.encoded_len());
    payment_ack.encode(&mut raw_ack).unwrap();

    Ok(Response::builder()
        .header(LOCATION, format!("/{}/{}", METADATA_PATH, addr_str))
        .header(AUTHORIZATION, token)
        .body(Body::from(raw_ack))
        .unwrap())
}

#[derive(Debug)]
pub enum PaymentRequestError {
    IncorrectLengthPreimage,
    Address(CashAddrError, Base58Error),
    Node(NodeError),
    UnepxectedNetwork,
    Hex(hex::FromHexError),
}

impl fmt::Display for PaymentRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(cash_err, base58_err) => {
                f.write_str(&format!("{}, {}", cash_err, base58_err))
            }
            Self::Hex(err) => err.fmt(f),
            Self::Node(err) => err.fmt(f),
            Self::UnepxectedNetwork => f.write_str("unexpected network"),
            Self::IncorrectLengthPreimage => f.write_str("incorrect length preimage"),
        }
    }
}

impl Reject for PaymentRequestError {}

impl IntoResponse for PaymentRequestError {
    fn to_status(&self) -> u16 {
        match self {
            Self::Address(_, _) => 400,
            Self::Hex(_) => 400,
            Self::IncorrectLengthPreimage => 400,
            Self::Node(err) => match err {
                NodeError::Rpc(_) => 400,
                _ => 500,
            },
            Self::UnepxectedNetwork => 400,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CommitQuery {
    address: String,
    metadata_digest: String,
}

pub async fn commit(query: CommitQuery) -> Result<Response<Body>, PaymentRequestError> {
    // Parse query
    let addr =
        address_decode(&query.address).map_err(|err| PaymentRequestError::Address(err.0, err.1))?;
    let metadata_digest_raw =
        hex::decode(query.metadata_digest).map_err(PaymentRequestError::Hex)?;

    // Generate output
    let commitment_preimage = [addr.as_body(), &metadata_digest_raw].concat();
    let commitment = Sha256::digest(&commitment_preimage);
    let op_return_pre: [u8; 2] = [106, COMMITMENT_SIZE as u8];
    let script = [&op_return_pre[..], commitment.as_slice()].concat();
    let output = Output {
        amount: Some(SETTINGS.payments.token_fee),
        script,
    };

    // Valid interval
    let current_time = SystemTime::now();
    let expiry_time = current_time + Duration::from_millis(SETTINGS.payments.timeout);

    let payment_details = PaymentDetails {
        network: Some(SETTINGS.network.to_string()),
        time: current_time.duration_since(UNIX_EPOCH).unwrap().as_secs(),
        expires: Some(expiry_time.duration_since(UNIX_EPOCH).unwrap().as_secs()),
        memo: None,
        merchant_data: Some(commitment_preimage),
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
