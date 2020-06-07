from requests import get, put, post, Response

KEYS_URL = "{}/keys/{}"
PAYMENTS_URL = "{}/payments"
PEERS_URL = "{}/peers"

class KeyserverClient:
    def __init__(self, url: str):
        self.url = url

    def commit(self, pubkey_digest: str, metadata_digest: bytes) -> Response:
        digest_hex = metadata_digest.hex()
        response = post(url=self.url + "/commit", json={
            'pubkey_digest': pubkey_digest,
            'metadata_digest': digest_hex
        })
        return response

    def get_metadata(self, address: str):
        response = get(url=KEYS_URL.format(self.url, address))
        return response

    def put_metadata(self, address: str, raw_metadata: bytes, token: str):
        response = put(url=KEYS_URL.format(self.url, address), data=raw_metadata, headers={
            "Authorization": token
        })
        return response

    def put_metadata_no_token(self, address: str, raw_metadata: bytes) -> Response:
        response = put(url=KEYS_URL.format(self.url, address), data=raw_metadata)
        return response

    def send_payment(self, raw_payment: bytes) -> Response:
        headers = {
            "Content-Type": "application/bitcoincash-payment",
            "Accept": "application/bitcoincash-paymentack"
        }
        response = post(url=PAYMENTS_URL.format(self.url), data=raw_payment, headers=headers)
        return response

    def get_peers(self) -> Response:
        response = get(url=PEERS_URL.format(self.url))
        return response