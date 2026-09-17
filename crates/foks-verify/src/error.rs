//! Verifier errors and transition-rule diagnostics.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid FOKS protocol data: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("FOKS cryptographic verification failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("hostchain must contain at least one link")]
    EmptyHostchain,
    #[error("hostchain sequence is {received}, expected {expected}")]
    Sequence { expected: u64, received: u64 },
    #[error("hostchain genesis link has a previous hash")]
    GenesisHasPrevious,
    #[error("non-genesis hostchain link is missing its previous hash")]
    MissingPrevious,
    #[error("hostchain previous hash does not match the accepted tail")]
    PreviousMismatch,
    #[error("host identity changed within the hostchain")]
    HostChanged,
    #[error("the genesis host key did not sign the first link")]
    GenesisSigner,
    #[error("hostchain signature count is {signatures}, expected {keys}")]
    SignatureCount { signatures: usize, keys: usize },
    #[error("a non-genesis link used an inactive or revoked host key")]
    InactiveHostSigner,
    #[error("invalid TLS CA certificate")]
    InvalidCertificate,
    #[error("TLS CA certificate does not contain the EntityID's Ed25519 key")]
    CertificateKeyMismatch,
    #[error("hostchain contains no active delegated {0} key")]
    MissingDelegatedKey(&'static str),
    #[error("no active delegated {0} key verifies the object")]
    DelegatedSignature(&'static str),
    #[error("Merkle root commits to a different hostchain tail")]
    MerkleHostchainMismatch,
    #[error("invalid canonical probe service address")]
    InvalidProbeAddress,
    #[error("unsupported user chain: expected single eldest link")]
    UnsupportedUserChain,
    #[error("user-chain identity, host, or eldest binding mismatch")]
    UserBinding,
    #[error("user-chain username or device-name disclosure is invalid")]
    UserDisclosure,
    #[error("user-chain signature count is {0}, expected 2")]
    UserSignatureCount(usize),
    #[error("user chain references an unknown HEPK fingerprint")]
    MissingHepk,
    #[error("user Merkle root is not the trusted root")]
    UntrustedUserRoot,
    #[error("user Merkle proof is invalid")]
    UserMerkleProof,
    #[error("Merkle root rolled back from epoch {stored} to {received}")]
    MerkleRollback { stored: u64, received: u64 },
    #[error("Merkle root forked at epoch {0}")]
    MerkleFork(u64),
    #[error("Merkle historical response does not match the requested epochs")]
    MerkleHistoryShape,
    #[error("Merkle skip-pointer verification failed")]
    MerkleBackPointer,
    #[error("persisted Merkle evidence is incomplete or inconsistent")]
    PersistedMerkleEvidence,
    #[error("user chain sequence, previous hash, or location commitment is invalid")]
    UserChainContinuity,
    #[error("user chain transition {seqno} violates {rule}")]
    UserTransition {
        seqno: u64,
        rule: UserTransitionRule,
    },
    #[error("persisted user evidence does not reproduce its stored projection")]
    PersistedUserEvidence,
    #[error("team-chain identity, host, eldest, or transition binding mismatch")]
    TeamBinding,
    #[error("team-chain name disclosure is invalid")]
    TeamDisclosure,
    #[error("team chain sequence, previous hash, location, or Merkle proof is invalid")]
    TeamChainContinuity,
    #[error("team-chain signer is not an authorized active member")]
    TeamSigner,
    #[error("team-chain signature stack is invalid")]
    TeamSignatureStack,
    #[error("team-chain shared-key schedule is invalid")]
    TeamKeySchedule,
    #[error("team-chain roster transition is invalid")]
    TeamRoster,
    #[error("persisted team evidence does not reproduce its stored projection")]
    PersistedTeamEvidence,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UserTransitionRule {
    #[error("the one-member-change limit")]
    MultipleMemberChanges,
    #[error("the active-signer requirement")]
    UnknownSigner,
    #[error("shared-key generation, ordering, or HEPK rules")]
    SharedKeyRotation,
    #[error("device provisioning authorization or metadata rules")]
    Provisioning,
    #[error("device revocation authorization or key-rotation rules")]
    Revocation,
    #[error("the requirement to retain an owner device")]
    LastOwner,
    #[error("username-change or standalone-rotation metadata rules")]
    StandaloneChange,
    #[error("the requirement to retain devices and an owner shared key")]
    EmptyResult,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
