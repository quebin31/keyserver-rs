from unittest import TestCase
from utilities import *
from paymentrequest_pb2 import *
from keyserver_client import KeyserverClient
from copy import copy
from time import sleep

bitcoin_client = BitcoinClient("127.0.0.1", 18443)
keyserver_client_a = KeyserverClient("http://0.0.0.0:8080")
keyserver_client_b = KeyserverClient("http://0.0.0.0:8081")

"""
This test presumes that there are three keyservers A, B, C, that keyserver A has peers [keyserver_b], that
keyserver B has peers [keyserver_c], and keyserver C has no peers.
"""

class TestPop(TestCase):
    def put_metadata(self, keyserver_client):
        # Construct auth wrapper
        address, keypair = generate_random_keypair()
        metadata = construct_dummy_metadata()
        auth_wrapper, _ = construct_auth_wrapper(metadata, keypair)

        # Truncate
        auth_wrapper_truncated = copy(auth_wrapper)
        auth_wrapper_truncated.serialized_payload = b''
        raw_auth_wrapper_truncated = auth_wrapper_truncated.SerializeToString()

        # Construct pubkey_digest
        response = keyserver_client.put_metadata_no_token(
            address, raw_auth_wrapper_truncated)
        self.assertEqual(response.status_code, 402)

        # Parse PaymentRequest and PaymentDetails
        payment_request = PaymentRequest.FromString(response.content)
        payment_details_raw = payment_request.serialized_payment_details
        payment_details = PaymentDetails.FromString(payment_details_raw)

        # Generate Payment
        payment = bitcoin_client.generate_payment_from_payment_request(
            payment_details)
        payment_raw = payment.SerializeToString()

        # Send payment
        response = keyserver_client.send_payment(payment_raw)
        self.assertEqual(response.status_code, 200)
        payment_ack = PaymentACK.FromString(response.content)

        token = response.headers["Authorization"]

        raw_auth_wrapper = auth_wrapper.SerializeToString()
        response = keyserver_client.put_metadata(
            address, raw_auth_wrapper, token)
        self.assertEqual(response.status_code, 200)

        return raw_auth_wrapper, address


    def test_push_gossip(self):
        """Obtain a POP token, PUT with it then GET on other server"""

        raw_auth_wrapper, address = self.put_metadata(keyserver_client_a)

        # Generate one block
        bitcoin_client.generate_blocks(1)

        # Check that it's missing after one block
        response = keyserver_client_b.get_metadata(address)
        self.assertEqual(response.status_code, 404)

        # Generate another block
        bitcoin_client.generate_blocks(1)
        sleep(1)

        # Check that it's missing after one block
        response = keyserver_client_b.get_metadata(address)
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.content, raw_auth_wrapper)

    def test_pull_gossip(self):
        # Put metadata to keyserver B
        raw_auth_wrapper, address = self.put_metadata(keyserver_client_b)

        # Get from keyserver A
        response = keyserver_client_a.get_metadata(address)
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.content, raw_auth_wrapper)
