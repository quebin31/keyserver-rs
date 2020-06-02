from unittest import TestCase

from utilities import *


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



    def test_put_get_using_pop(self):
        """Obtain a POP token, PUT with it then GET"""

        # Construct auth wrapper
        address, keypair = generate_random_keypair()
        metadata = construct_dummy_metadata()
        auth_wrapper, digest = construct_auth_wrapper(metadata, keypair)

        # Construct pubkey_digest
        pubkey = keypair.get_pubkey()
        pubkey_digest = sha256(pubkey).hexdigest()
        response = self.keyserver_client.commit(pubkey_digest, digest)
        self.assertEqual(response.status_code, 402)

        