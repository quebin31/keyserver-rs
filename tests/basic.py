from unittest import TestCase
from utilities import *
from paymentrequest_pb2 import *
from keyserver_client import KeyserverClient

class TestPop(TestCase):
    def setUp(self):
        self.bitcoin_client = BitcoinClient("127.0.0.1", 18443)
        self.keyserver_client = KeyserverClient("http://0.0.0.0:8080")

    def test_get_missing(self):
        """Get random missing metadata."""

        address, _ = generate_random_keypair()
        response = self.keyserver_client.get_metadata(address)
        self.assertEqual(response.status_code, 404)

    def test_put_without_pop(self):
        """PUT without a POP token"""

        # Construct auth wrapper
        address, keypair = generate_random_keypair()
        metadata = construct_dummy_metadata()
        auth_wrapper, digest = construct_auth_wrapper(metadata, keypair)

        raw_auth_wrapper = auth_wrapper.SerializeToString()
        response = self.keyserver_client.put_metadata_no_token(address, raw_auth_wrapper)
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.text, "missing token")


    def test_put_get_using_pop(self):
        """Obtain a POP token, PUT with it then GET"""

        # Construct auth wrapper
        address, keypair = generate_random_keypair()
        metadata = construct_dummy_metadata()
        auth_wrapper, auth_wrapper_digest = construct_auth_wrapper(metadata, keypair)

        # Construct pubkey_digest
        pubkey = keypair.get_pubkey()
        pubkey_digest = sha256(pubkey).hexdigest()
        response = self.keyserver_client.commit(pubkey_digest, auth_wrapper_digest)
        self.assertEqual(response.status_code, 402)

        # Parse PaymentRequest and PaymentDetails
        payment_request = PaymentRequest.FromString(response.content)
        payment_details_raw = payment_request.serialized_payment_details
        payment_details = PaymentDetails.FromString(payment_details_raw)

        # Generate Payment
        payment = self.bitcoin_client.generate_payment_from_payment_request(payment_details)
        payment_raw = payment.SerializeToString()

        # Send payment
        response = self.keyserver_client.send_payment(payment_raw)
        self.assertEqual(response.status_code, 200)
        payment_ack = PaymentACK.FromString(response.content)

        token = response.headers["Authorization"]
        print(token)

        raw_metadata = auth_wrapper.SerializeToString()
        response = self.keyserver_client.put_metadata(address, raw_metadata, token)
        print(response.text)
        self.assertEqual(response.status_code, 200)