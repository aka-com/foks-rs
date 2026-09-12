//! The pinned Go token is 26 bytes: three public name bytes and 23 secret bytes.
use foks_proto::{decode_base62_strict, encode_base62_strict, SecretSeed, ENTITY_BOT_TOKEN_KEY};
use zeroize::Zeroizing;

pub const BOT_TOKEN_SEED_TYPE_ID: u64 = 0xe701_048d_6c6b_a4ba;
pub struct BotToken(Zeroizing<[u8; 26]>);
impl std::fmt::Debug for BotToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BotToken([REDACTED])")
    }
}
impl BotToken {
    pub fn generate() -> crate::Result<Self> {
        let mut seed = Zeroizing::new([0; 26]);
        getrandom::fill(&mut *seed).map_err(|_| crate::Error::DeviceKey)?;
        Ok(Self(seed))
    }
    pub fn from_seed(seed: [u8; 26]) -> Self {
        Self(Zeroizing::new(seed))
    }
    /// Caller owns clearing the input. Parsing never includes it in errors.
    pub fn import(input: &str) -> crate::Result<Self> {
        if input.len() != 37 || input.as_bytes()[5] != b'.' {
            return Err(crate::Error::DeviceKey);
        }
        let mut name = 0u32;
        for b in &input.as_bytes()[..5] {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'z' => b - b'a' + 10,
                _ => return Err(crate::Error::DeviceKey),
            };
            name = name * 36 + u32::from(digit);
        }
        if name > 0xff_ffff {
            return Err(crate::Error::DeviceKey);
        }
        let secret = Zeroizing::new(decode_base62_strict(&input[6..])?);
        if secret.len() != 23 {
            return Err(crate::Error::DeviceKey);
        }
        let mut seed = Zeroizing::new([0; 26]);
        seed[..3].copy_from_slice(&name.to_be_bytes()[1..]);
        seed[3..].copy_from_slice(&secret);
        Ok(Self(seed))
    }
    pub fn name(&self) -> String {
        let mut name = u32::from_be_bytes([0, self.0[0], self.0[1], self.0[2]]);
        let mut chars = [b'0'; 5];
        for c in chars.iter_mut().rev() {
            *c = b"0123456789abcdefghijklmnopqrstuvwxyz"[(name % 36) as usize];
            name /= 36;
        }
        String::from_utf8(chars.to_vec()).expect("ASCII alphabet")
    }
    pub fn export(&self) -> Zeroizing<String> {
        let secret = Zeroizing::new(encode_base62_strict(&self.0[3..]));
        Zeroizing::new(format!("{}.{}", self.name(), secret.as_str()))
    }
    /// Go's KDF is host independent; enrollment and certificates bind authority to a host.
    pub fn derived_seed(&self) -> SecretSeed {
        let mut encoded = Zeroizing::new([0; 28]);
        encoded[..2].copy_from_slice(&[0xc4, 26]);
        encoded[2..].copy_from_slice(&*self.0);
        SecretSeed::new(
            crate::prefixed_hash_signable(BOT_TOKEN_SEED_TYPE_ID, &*encoded)
                .expect("canonical fixed bin8"),
        )
    }
    pub fn public_material(&self) -> crate::Result<crate::DevicePublicMaterial> {
        crate::derive_public_material(&self.derived_seed(), ENTITY_BOT_TOKEN_KEY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_vectors_and_strict_import() {
        let root = "../foks-snowpack/tests/fixtures/foks-v0.1.9/account";
        for n in ["zero", "incremental", "high"] {
            let read = |suffix: &str| std::fs::read(format!("{root}/bot-{n}.{suffix}")).unwrap();
            let token = BotToken::from_seed(read("seed").try_into().unwrap());
            assert_eq!(token.export().as_bytes(), read("txt"));
            assert_eq!(token.derived_seed().as_slice(), read("derived"));
            assert_eq!(token.public_material().unwrap().id.as_bytes(), read("id"));
            assert_eq!(
                token.public_material().unwrap().hepk.encoded().unwrap(),
                read("hepk")
            );
            assert_eq!(
                *BotToken::import(&token.export()).unwrap().export(),
                *token.export()
            );
        }
        for bad in [
            "",
            "00000.000000000000000000000000000000 ",
            "zzzzz.0000000000000000000000000000000",
            "00000.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "00000.000000000000000000000000000000é",
        ] {
            assert!(BotToken::import(bad).is_err());
        }
        assert_eq!(
            format!("{:?}", BotToken::from_seed([0; 26])),
            "BotToken([REDACTED])"
        );
    }
}
