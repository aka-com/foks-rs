//! Go v0.1.9 realtime encryption. The caller supplies authenticated PTK material.
use crate::hybrid::derive_key;
use crate::primitives::{
    open_typed_secretbox, prefixed_hash_signable, seal_typed_secretbox, typed_hmac,
};
use crate::{Error, Result};
use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
use foks_proto::{
    RealtimeWire, RtAppId, RtCiphertext, RtKeyType, RtMessageBody, RtMessageNoncer, RtMessageType,
    RtText, RT_CHANNEL_DESC_TYPE_ID, RT_CHANNEL_NAME_TYPE_ID, RT_KEY_DERIVATION_TYPE_ID,
    RT_MAX_BODY_BYTES, RT_MAX_CIPHERTEXT_BYTES, RT_MSG_NONCER_TYPE_ID,
};
use foks_proto::{SecretBox, SecretSeed, APP_KEY_DERIVATION_TYPE_ID};
use foks_snowpack::{encode, Value};
use zeroize::Zeroizing;

/// App-scoped keys; deliberately neither Debug nor serializable.
pub struct RealtimeKeys {
    app: RtAppId,
    name: Zeroizing<[u8; 32]>,
    description: Zeroizing<[u8; 32]>,
    data: Zeroizing<[u8; 32]>,
}

pub fn derive_realtime_keys(seed: &SecretSeed, app: RtAppId) -> Result<RealtimeKeys> {
    if app == RtAppId::None {
        return Err(Error::Realtime);
    }
    let app_key = derive_key(seed, 5, None)?;
    let realtime = Zeroizing::new(typed_hmac(
        app_key.as_slice(),
        APP_KEY_DERIVATION_TYPE_ID,
        &encode(&Value::Array(vec![
            Value::Unsigned(0),
            Value::Variant(Some((b"0".to_vec(), Box::new(Value::Unsigned(1))))),
        ]))?,
    ));
    let key = |purpose: RtKeyType| -> Result<Zeroizing<[u8; 32]>> {
        Ok(Zeroizing::new(typed_hmac(
            realtime.as_ref(),
            RT_KEY_DERIVATION_TYPE_ID,
            &encode(&Value::Array(vec![
                app.to_value(),
                Value::Array(vec![purpose.to_value(), Value::Variant(None)]),
            ]))?,
        )))
    };
    Ok(RealtimeKeys {
        app,
        name: key(RtKeyType::ChannelName)?,
        description: key(RtKeyType::ChannelDescription)?,
        data: key(RtKeyType::Data)?,
    })
}

pub fn realtime_message_nonce(noncer: &RtMessageNoncer) -> Result<[u8; 24]> {
    if !matches!(
        noncer.team.party.entity_type(),
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) || noncer.team.host.entity_type() != foks_proto::ENTITY_HOST
    {
        return Err(Error::Realtime);
    }
    let hash = prefixed_hash_signable(RT_MSG_NONCER_TYPE_ID, &noncer.encoded()?)?;
    Ok(hash[..24].try_into().expect("24-byte hash prefix"))
}

impl RealtimeKeys {
    fn text_key(&self, purpose: RtKeyType) -> Result<(&[u8; 32], u64)> {
        match purpose {
            RtKeyType::ChannelName => Ok((&self.name, RT_CHANNEL_NAME_TYPE_ID)),
            RtKeyType::ChannelDescription => Ok((&self.description, RT_CHANNEL_DESC_TYPE_ID)),
            RtKeyType::Data => Err(Error::Realtime),
        }
    }

    /// Randomness is supplied by the caller, allowing deterministic oracle tests.
    pub fn seal_text(
        &self,
        purpose: RtKeyType,
        text: &RtText,
        nonce: [u8; 16],
    ) -> Result<SecretBox> {
        if text.0.len() > RT_MAX_BODY_BYTES {
            return Err(Error::Realtime);
        }
        let (key, type_id) = self.text_key(purpose)?;
        let plaintext = Zeroizing::new(text.encoded()?);
        let ciphertext = seal_typed_secretbox(key, type_id, &nonce, &plaintext, false)
            .map_err(|_| Error::Realtime)?;
        Ok(SecretBox { nonce, ciphertext })
    }

    pub fn open_text(&self, purpose: RtKeyType, boxed: &SecretBox) -> Result<RtText> {
        if boxed.ciphertext.len() > RT_MAX_CIPHERTEXT_BYTES {
            return Err(Error::Realtime);
        }
        let (key, type_id) = self.text_key(purpose)?;
        let plaintext = open_typed_secretbox(key, type_id, &boxed.nonce, &boxed.ciphertext)?;
        Ok(RtText::decode(&plaintext)?)
    }

    pub fn seal_basic_message(
        &self,
        noncer: &RtMessageNoncer,
        body: &[u8],
    ) -> Result<RtCiphertext> {
        if noncer.metadata.kind != RtMessageType::Basic || body.len() > RT_MAX_BODY_BYTES {
            return Err(Error::Realtime);
        }
        self.seal_message(noncer, &RtMessageBody::Basic(body.to_vec()))
    }

    /// Seal a Basic message or a pegged Reply, Reaction or Edit as go-foks
    /// main's `sealMessage` does (bf9ace5). The body arm must match the
    /// metadata type, and a pegged body must name a non-zero target.
    pub fn seal_message(
        &self,
        noncer: &RtMessageNoncer,
        body: &RtMessageBody,
    ) -> Result<RtCiphertext> {
        self.check_sender(noncer)?;
        let (kind, target) = match body {
            RtMessageBody::Basic(_) => (RtMessageType::Basic, None),
            RtMessageBody::Pegged { kind, reply_to, .. } => (*kind, Some(reply_to)),
        };
        if kind != noncer.metadata.kind || target.is_some_and(|id| id.0 == [0; 16]) {
            return Err(Error::Realtime);
        }
        let plaintext = Zeroizing::new(body.encoded()?);
        let nonce = realtime_message_nonce(noncer)?;
        let ciphertext = XSalsa20Poly1305::new((&*self.data).into())
            .encrypt((&nonce).into(), plaintext.as_ref())
            .map_err(|_| Error::Realtime)?;
        Ok(RtCiphertext(ciphertext))
    }

    pub fn open_basic_message(
        &self,
        noncer: &RtMessageNoncer,
        ciphertext: &RtCiphertext,
    ) -> Result<Zeroizing<Vec<u8>>> {
        if noncer.metadata.kind != RtMessageType::Basic {
            return Err(Error::Realtime);
        }
        match self.open_message(noncer, ciphertext)? {
            RtMessageBody::Basic(ref body) => Ok(Zeroizing::new(body.clone())),
            _ => Err(Error::Realtime),
        }
    }

    /// Open a Basic message or a pegged Reply, Reaction or Edit (go-foks
    /// bf9ace5). The decrypted body arm must match the message type in the
    /// metadata: the server stores that type unauthenticated, so a mismatch
    /// would let a sender show readers one type while sealing another.
    pub fn open_message(
        &self,
        noncer: &RtMessageNoncer,
        ciphertext: &RtCiphertext,
    ) -> Result<RtMessageBody> {
        self.check_sender(noncer)?;
        if !matches!(
            noncer.metadata.kind,
            RtMessageType::Basic
                | RtMessageType::Edit
                | RtMessageType::Reaction
                | RtMessageType::Reply
        ) {
            return Err(Error::Realtime);
        }
        if ciphertext.0.len() > RT_MAX_CIPHERTEXT_BYTES {
            return Err(Error::Realtime);
        }
        let nonce = realtime_message_nonce(noncer)?;
        let plaintext = Zeroizing::new(
            XSalsa20Poly1305::new((&*self.data).into())
                .decrypt((&nonce).into(), ciphertext.0.as_ref())
                .map_err(|_| Error::Decryption)?,
        );
        let body = RtMessageBody::decode(&plaintext)?;
        let kind = match &body {
            RtMessageBody::Basic(_) => RtMessageType::Basic,
            RtMessageBody::Pegged { kind, .. } => *kind,
        };
        if kind != noncer.metadata.kind {
            return Err(Error::Realtime);
        }
        Ok(body)
    }

    fn check_sender(&self, noncer: &RtMessageNoncer) -> Result<()> {
        if noncer.app != self.app || noncer.sender.is_none() || noncer.metadata.id.0 == [0; 16] {
            return Err(Error::Realtime);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kv::derive_kv_keys;
    #[test]
    fn purpose_keys_match_go_oracle() {
        let seed = SecretSeed::new(std::array::from_fn(|i| i as u8));
        let keys = derive_realtime_keys(&seed, RtAppId::Chat).unwrap();
        let kv = derive_kv_keys(&seed).unwrap();
        let crdt = derive_realtime_keys(&seed, RtAppId::Crdt).unwrap();
        assert_ne!(keys.data.as_slice(), kv.box_key.as_slice());
        assert_ne!(keys.data.as_slice(), crdt.data.as_slice());
        assert_ne!(keys.name.as_slice(), keys.description.as_slice());
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime"
        );
        for (index, key) in [(1, &keys.name), (2, &keys.description), (3, &keys.data)] {
            assert_eq!(
                key.as_slice(),
                std::fs::read(format!("{root}/key-{index}.bin")).unwrap()
            );
        }
    }
}
