use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitcoincash_addr::{cashaddr::EncodingError as AddrEncodingError, Address};
use cashweb::bitcoin_client::{BitcoinClient, HttpClient, HttpError, NodeError};
use cashweb::{
    bitcoin::{
        transaction::{DecodeError as TransactionDecodeError, Transaction},
        Decodable,
    },
    payments::{bip70::*, PreprocessingError},
    token::schemes::chain_commitment::*,
};
use prost::Message as _;
use ring::digest::{digest, SHA256};
use warp::{
    http::{
        header::{AUTHORIZATION, LOCATION},
        Response,
    },
    hyper::Body,
    reject::Reject,
};

use super::IntoResponse;
use crate::{METADATA_PATH, PAYMENTS_PATH, SETTINGS};

pub const COMMITMENT_PREIMAGE_SIZE: usize = 32 + 32;
pub const COMMITMENT_SIZE: usize = 32;
pub const OP_RETURN: u8 = 106;

#[derive(Debug)]
pub enum PaymentError {
    Preprocess(PreprocessingError),
    MissingCommitment,
    MalformedTx(TransactionDecodeError),
    MissingMerchantData,
    Node(HttpError),
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
    bitcoin_client: BitcoinClient<HttpClient>,
) -> Result<Response<Body>, PaymentError> {
    // Deserialize transactions
    let txs_res: Result<Vec<(Transaction, Vec<u8>)>, _> = payment
        .transactions
        .iter()
        .map(|raw_tx| {
            let mut tx_id = digest(&SHA256, digest(&SHA256, &raw_tx).as_ref())
                .as_ref()
                .to_vec();
            tx_id.as_mut_slice().reverse();
            Transaction::decode(&mut raw_tx.as_slice()).map(move |tx| (tx, tx_id))
        })
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
    let pub_key_hash = &commitment_preimage[..32];
    let address = Address {
        body: pub_key_hash.to_vec(),
        ..Default::default()
    };
    let addr_str = address.encode().map_err(PaymentError::Address)?;

    // Extract metadata
    let address_metadata_hash = &commitment_preimage[32..COMMITMENT_PREIMAGE_SIZE];

    let expected_commitment = construct_commitment(pub_key_hash, address_metadata_hash);

    let (tx_id, vout) = txs
        .iter()
        .find_map(|(tx, tx_id)| {
            tx.outputs
                .iter()
                .enumerate()
                .find_map(|(vout, output)| {
                    let raw_script = output.script.as_bytes();
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
                .map(|vout| (tx_id, vout))
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
    Node(HttpError),
    UnepxectedNetwork,
    PubkeyDigestHex(hex::FromHexError),
    MetadataDigestHex(hex::FromHexError),
}

impl fmt::Display for PaymentRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PubkeyDigestHex(err) => {
                f.write_str(&format!("public key digest failed to decodel {}", err))
            }
            Self::MetadataDigestHex(err) => {
                f.write_str(&format!("metadata digest failed to decodel {}", err))
            }
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
            Self::PubkeyDigestHex(_) => 400,
            Self::IncorrectLengthPreimage => 400,
            Self::MetadataDigestHex(_) => 400,
            Self::Node(err) => match err {
                NodeError::Rpc(_) => 400,
                _ => 500,
            },
            Self::UnepxectedNetwork => 400,
        }
    }
}

pub fn construct_payment_response(pub_key_hash: &[u8], metadata_digest: &[u8]) -> Response<Body> {
    // Construct metadata commitment
    let commitment_preimage = [pub_key_hash, metadata_digest].concat();
    let commitment = digest(&SHA256, &commitment_preimage);
    let op_return_pre: [u8; 2] = [106, COMMITMENT_SIZE as u8];
    let script = [&op_return_pre[..], commitment.as_ref()].concat();
    let output = Output {
        amount: None,
        script,
    };

    // Valid interval
    let current_time = SystemTime::now();

    let payment_details = PaymentDetails {
        network: Some(SETTINGS.network.to_string()),
        time: current_time.duration_since(UNIX_EPOCH).unwrap().as_secs(),
        expires: None,
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

    Response::builder()
        .status(402)
        .body(Body::from(payment_invoice_raw))
        .unwrap()
}
