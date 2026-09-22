use serde::{Deserialize, Serialize};
use zeroize::Zeroize;
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SsoSessionPayload {
    pub id: [u8; 17],
    pub verifier: String,
    pub nonce: String,
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub issuer: String,
    pub subject: String,
    pub username: String,
    pub expires_at_ms: u64,
    pub binding_hash: Option<[u8; 32]>,
    pub bound_uid: Option<Vec<u8>>,
    pub email: Option<String>,
    pub poll_reservation: Option<Vec<u8>>,
}
impl Drop for SsoSessionPayload {
    fn drop(&mut self) {
        self.id.zeroize();
        self.verifier.zeroize();
        self.nonce.zeroize();
        self.access_token.zeroize();
        self.id_token.zeroize();
        self.refresh_token.zeroize();
        self.poll_reservation.zeroize();
    }
}
