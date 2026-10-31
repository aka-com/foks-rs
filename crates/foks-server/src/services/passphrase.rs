//! Owned copies of the exact passphrase-box bytes that signup, user-chain
//! mutations and generic-chain passphrase updates persist.

use crate::Result;

/// The passphrase material of a signup, user mutation or passphrase update,
/// with each box kept as the exact encoded bytes the client submitted.
pub(crate) struct OwnedPassphraseMutation {
    verify_key: Vec<u8>,
    /// A user mutation replaces the submitted salt with the stored one.
    pub(super) salt: [u8; 16],
    generation: u64,
    exact_skmwk_box: Vec<u8>,
    exact_passphrase_box: Vec<u8>,
    exact_puk_box: Option<Vec<u8>>,
    puk_generation: Option<u64>,
    puk_role: Option<foks_proto::Role>,
    stretch_version: u64,
}

impl OwnedPassphraseMutation {
    pub(crate) fn from_argument(argument: &foks_proto::PassphraseUpdateArgument) -> Result<Self> {
        Ok(Self {
            verify_key: argument.verify_key.as_bytes().to_vec(),
            salt: argument.salt,
            generation: argument.generation,
            exact_skmwk_box: foks_snowpack::encode(&argument.skmwk_box.to_value())?,
            exact_passphrase_box: argument.passphrase_box.encoded()?,
            exact_puk_box: argument
                .puk_box
                .as_ref()
                .map(foks_proto::PpePukBox::encoded)
                .transpose()?,
            puk_generation: argument.puk_box.as_ref().map(|boxed| boxed.puk_generation),
            puk_role: argument.puk_box.as_ref().map(|boxed| boxed.puk_role),
            stretch_version: argument.stretch_version.protocol_value(),
        })
    }

    pub(crate) fn as_database(&self, now: u64) -> foks_server_db::PassphraseMutation<'_> {
        foks_server_db::PassphraseMutation {
            verify_key: &self.verify_key,
            salt: &self.salt,
            generation: self.generation,
            exact_skmwk_box: &self.exact_skmwk_box,
            exact_passphrase_box: &self.exact_passphrase_box,
            exact_puk_box: self.exact_puk_box.as_deref(),
            puk_generation: self.puk_generation,
            puk_role: self.puk_role,
            stretch_version: self.stretch_version,
            now,
        }
    }
}
