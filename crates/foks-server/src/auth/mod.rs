mod principal;
pub(crate) mod team;

#[cfg(feature = "celld-proof")]
pub(crate) use principal::parse_hosted_peer_certificate;
pub(crate) use principal::Principal;
