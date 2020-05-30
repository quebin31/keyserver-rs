pub mod database {
    include!(concat!(env!("OUT_DIR"), "/database.rs"));
}

pub mod keyserver {
    include!(concat!(env!("OUT_DIR"), "/keyserver.rs"));
}

// TODO: Move to cahsweb-rs
pub mod wrapper {
    use secp256k1::{key::PublicKey, Error as SecpError, Message, Secp256k1, Signature};
    use sha2::{Digest, Sha256};

    include!(concat!(env!("OUT_DIR"), "/wrapper.rs"));

    #[derive(Debug)]
    pub enum ValidationError {
        NotFound,
        InvalidSignature(SecpError),
        Message(SecpError),
        MetadataDecode(prost::DecodeError),
        PublicKey(SecpError),
        Signature(SecpError),
        UnsupportedScheme,
    }

    impl AuthWrapper {
        pub fn validate(&self) -> Result<(), ValidationError> {
            let pubkey =
                PublicKey::from_slice(&self.pub_key).map_err(ValidationError::PublicKey)?;
            if self.scheme != 1 {
                // TODO: Support Schnorr
                return Err(ValidationError::UnsupportedScheme);
            }
            let signature =
                Signature::from_compact(&self.signature).map_err(ValidationError::Signature)?;
            let secp = Secp256k1::verification_only();
            let payload_digest = Sha256::digest(&self.serialized_payload);
            let msg = Message::from_slice(&payload_digest).map_err(ValidationError::Message)?;
            secp.verify(&msg, &signature, &pubkey)
                .map_err(ValidationError::InvalidSignature)?;
            Ok(())
        }
    }
}
