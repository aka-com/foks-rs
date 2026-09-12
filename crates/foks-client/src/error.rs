//! Client orchestration, transport, and binding errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid chat input: {0}")]
    ChatInvalidInput(&'static str),
    #[error("unsupported chat operation: {0}")]
    ChatUnsupported(&'static str),
    #[error("chat access denied: {0}")]
    ChatAccessDenied(&'static str),
    #[error("refresh chat state before retrying: {0}")]
    ChatRefreshRequired(&'static str),
    #[error("cancel this preparation and prepare a new chat operation: {0}")]
    ChatReprepareRequired(&'static str),
    #[error("chat object unavailable: {0}")]
    ChatNotFound(&'static str),
    #[error("verified chat key unavailable: {0}")]
    ChatKeyUnavailable(&'static str),
    #[error("chat capacity exceeded: {0}")]
    ChatLimit(&'static str),
    #[error("chat operation cannot make this transition: {0}")]
    ChatOperationState(&'static str),
    #[error("chat channel name conflict: {0}")]
    ChatNameConflict(&'static str),
    #[error("chat randomness failed: {0}")]
    ChatRandomness(&'static str),
    #[error("chat integrity check failed: {0}")]
    ChatIntegrity(&'static str),
    /// Message evidence failed after account and channel metadata verification.
    #[error("chat channel content integrity check failed: {0}")]
    ChatChannelIntegrity(&'static str),
    #[error("invalid probe target: {0}")]
    Target(&'static str),
    #[error("DNS lookup returned no addresses for {0}")]
    NoAddress(String),
    #[error("TCP connection to every resolved address failed: {0}")]
    Connect(std::io::Error),
    #[error("FOKS operation cancelled")]
    Cancelled,
    #[error("FOKS operation deadline exceeded")]
    DeadlineExceeded,
    #[error("FOKS transport configuration failed: {0}")]
    Transport(&'static str),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("invalid TLS server name")]
    ServerName,
    #[error("FOKS RPC failed: {0}")]
    Rpc(#[from] foks_rpc::Error),
    #[error("FOKS public state verification failed: {0}")]
    Verify(#[from] foks_verify::Error),
    #[error("FOKS hard-state update failed: {0}")]
    Database(#[from] foks_client_db::Error),
    #[error("invalid FOKS protocol value: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("FOKS device cryptography failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("invalid FOKS backup key: {0}")]
    Backup(#[from] foks_crypto::BackupPhraseError),
    #[error("invalid FOKS KEX phrase: {0}")]
    KexPhrase(#[from] foks_crypto::KexPhraseError),
    #[error("FOKS KEX failed: {0}")]
    Kex(&'static str),
    #[error("registration returned an invalid certificate chain")]
    CertificateChain,
    #[error("authenticated hostchain contains no usable TLS CA certificates")]
    HostTlsRoots,
    #[error("invalid FOKS user identifier")]
    InvalidUserId,
    #[error("FOKS credential binding failed: {0}")]
    CredentialBinding(&'static str),
    #[error("FOKS pinned-host binding failed: {0}")]
    HostBinding(&'static str),
    #[error("FOKS federation discovery failed: {0}")]
    FederationDiscovery(&'static str),
    #[error("FOKS user-state binding failed: {0}")]
    UserBinding(&'static str),
    #[error("FOKS team-state binding failed: {0}")]
    TeamBinding(&'static str),
    #[error("FOKS private-key binding failed: {0}")]
    KeyBinding(&'static str),
    #[error("FOKS mutation-journal binding failed: {0}")]
    OperationBinding(&'static str),
    #[error("FOKS protected mutation material failed: {0}")]
    ProtectedMaterial(String),
    #[error("FOKS transition was not observed: {0}")]
    TransitionNotObserved(&'static str),
    #[error("pinned host is missing or has a malformed {0} service endpoint")]
    PinnedService(&'static str),
    #[error("invalid or excessive FOKS KV response: {0}")]
    KvResponse(&'static str),
    #[error("invalid FOKS KV request: {0}")]
    KvRequest(&'static str),
    #[error("invalid FOKS account or user request: {0}")]
    AccountRequest(&'static str),
    #[error("invalid FOKS team request: {0}")]
    TeamRequest(&'static str),
    #[error("FOKS scheduler error: {0}")]
    Scheduler(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
