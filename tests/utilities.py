import bitcoin
import os
from requests import get, put, post, Response

from bitcoinrpc.authproxy import AuthServiceProxy, JSONRPCException
from bitcoin.core.key import CECKey
from bitcoin.wallet import P2PKHBitcoinAddress
from bitcoin.core import CMutableTransaction, CMutableTxIn
from keyserver_pb2 import *
from paymentrequest_pb2 import *
from wrapper_pb2 import *
from hashlib import sha256
from time import time
from decimal import Decimal

SATS_PER_BITCOIN = 100_000_000

class BitcoinClient:
    sats_per_byte = Decimal(1) / 10_000_000

    def __init__(self, url, port, username="user", password="password"):
        bitcoin.SelectParams("regtest")

        # Init Bitcoin RPC
        url = "http://%s:%s@%s:%s" % (username, password, url, port)
        rpc_connection = AuthServiceProxy(url)

        self.connection = rpc_connection

    def collect_inputs(self, amount: Decimal, n_bytes=200):
        """Collect inputs up to a specific amount and return them and total satoshis."""

        utxos = self.connection.listunspent()
        input_value = Decimal(0)
        inputs = []
        for utxo in utxos:
            if input_value < amount + n_bytes * self.sats_per_byte:
                inputs.append({
                    "txid": utxo["txid"],
                    "vout": utxo["vout"]
                })
                input_value += utxo["amount"]
            else:
                break
        change_address = utxo['address']
        return inputs, input_value, change_address

    def construct_op_return(self, data: bytes) -> bytes:
        """Construct transaction from OP_RETURN data"""

        # Collect inputs and outputs
        amount = Decimal(5_000) / SATS_PER_BITCOIN
        inputs, input_amount, change_address = self.collect_inputs(amount)
        outputs = [
            {"data": data.hex()},
            {change_address: input_amount - amount}
        ]

        # Construct and sign transaction
        raw_tx_unsigned = self.connection.createrawtransaction(inputs, outputs)
        signed_tx = self.connection.signrawtransactionwithwallet(
            raw_tx_unsigned)
        raw_tx = bytes.fromhex(signed_tx["hex"])
        return raw_tx

    def generate_tx_from_payment_request(self, payment_details: PaymentDetails) -> bytes:
        """Construct transaction from payment request."""

        op_return = payment_details.outputs[0].script[2:]
        raw_tx = self.construct_op_return(op_return)
        return raw_tx

    def generate_payment_from_payment_request(self, payment_details: PaymentDetails) -> Payment:
        """Construct Payment message."""

        raw_tx = self.generate_tx_from_payment_request(payment_details)
        payment = Payment(merchant_data=payment_details.merchant_data,
                  transactions=[raw_tx])
        return payment

def construct_client():
    """Construct bitcoin client connected to registry test."""
    bitcoin.SelectParams("regtest")

    # Init Bitcoin RPC
    rpc_user = "user"
    rpc_password = "password"
    rpc_connection = AuthServiceProxy(
        "http://%s:%s@127.0.0.1:18443" % (rpc_user, rpc_password))

    return rpc_connection


def generate_random_keypair():
    """Generate a random bitcoin address, a ECDSA keypair."""

    # Generate keys
    secret = os.urandom(16)
    keypair = CECKey()
    keypair.set_compressed(True)
    keypair.set_secretbytes(secret)
    public_key = keypair.get_pubkey()

    # Generate key addr
    key_addr = str(P2PKHBitcoinAddress.from_pubkey(public_key))
    return key_addr, keypair


def construct_dummy_metadata() -> AddressMetadata:
    """Construct some dummy metadata."""

    header = Header(name="Something wicked", value="this way comes")
    entry = Entry(headers=[header],
                  entry_data=b'This gonna be very fast')
    timestamp = int(time())
    metadata = AddressMetadata(timestamp=timestamp, ttl=3000, entries=[entry])
    return metadata


def sign_metadata(raw_metadata: bytes, keypair: CECKey):
    """Return the AddressMetadata digest and a signature over it"""
    digest = sha256(raw_metadata).digest()
    signature, _ = keypair.sign_compact(digest)
    return signature, digest


def construct_auth_wrapper(metadata: AddressMetadata, keypair: CECKey):
    """Return the complete AuthWrapper object and the digest of the metadata."""
    raw_metadata = metadata.SerializeToString()
    signature, digest = sign_metadata(raw_metadata, keypair)
    public_key = keypair.get_pubkey()
    auth_wrapper = AuthWrapper(
        pub_key=public_key, serialized_payload=raw_metadata, scheme=1, signature=signature)
    return auth_wrapper, digest
