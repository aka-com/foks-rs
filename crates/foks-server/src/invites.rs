use std::path::Path;

use foks_proto::InviteCode;

use crate::{Error, Result};

const INVITE_CODE_HASH_TYPE_ID: u64 = 0xd6d9_3407_464f_4b53;

#[derive(Clone, Eq, PartialEq)]
pub struct IssuedSignupInvite {
    pub code: String,
    pub record: foks_server_db::IssuedInvite,
}

impl std::fmt::Debug for IssuedSignupInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedSignupInvite")
            .field("code", &"<redacted>")
            .field("record", &self.record)
            .finish()
    }
}

pub fn invite_fingerprint(code: &InviteCode) -> Result<[u8; 32]> {
    code.validate()?;
    Ok(foks_crypto::prefixed_hash(
        INVITE_CODE_HASH_TYPE_ID,
        &code.encoded()?,
    ))
}

pub fn invite_kind(code: &InviteCode) -> Result<foks_server_db::InviteKind> {
    match code {
        InviteCode::Standard(_) => Ok(foks_server_db::InviteKind::Standard),
        InviteCode::MultiUse(_) => Ok(foks_server_db::InviteKind::MultiUse),
        _ => Err(Error::Config("unsupported signup invite kind")),
    }
}

pub fn issue_standard_invite(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    expires_at: Option<u64>,
    now: u64,
) -> Result<IssuedSignupInvite> {
    let mut raw = [0u8; 10];
    getrandom::fill(&mut raw).map_err(|_| Error::Config("OS randomness unavailable"))?;
    issue_invite(
        database_path,
        database_config,
        InviteCode::Standard(raw.to_vec()),
        Some(1),
        expires_at,
        now,
    )
}

pub fn issue_multiuse_invite(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    code: &str,
    max_uses: Option<u64>,
    expires_at: Option<u64>,
    now: u64,
) -> Result<IssuedSignupInvite> {
    issue_invite(
        database_path,
        database_config,
        InviteCode::from_user_input(code, false)?,
        max_uses,
        expires_at,
        now,
    )
}

fn issue_invite(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    code: InviteCode,
    max_uses: Option<u64>,
    expires_at: Option<u64>,
    now: u64,
) -> Result<IssuedSignupInvite> {
    let hash = invite_fingerprint(&code)?;
    let kind = invite_kind(&code)?;
    let mut invite_id = [0u8; 16];
    getrandom::fill(&mut invite_id).map_err(|_| Error::Config("OS randomness unavailable"))?;
    let display = code.to_user_string()?;
    let guard = crate::DatabaseWriterGuard::acquire(database_path.as_ref())?;
    let mut database = guard.open_database(database_config)?;
    let record = database.issue_invite(&invite_id, &hash, kind, None, max_uses, expires_at, now)?;
    Ok(IssuedSignupInvite {
        code: display,
        record,
    })
}

pub fn disable_invite(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    code: &str,
    now: u64,
) -> Result<bool> {
    let code = InviteCode::from_user_input(code, false)?;
    let hash = invite_fingerprint(&code)?;
    let guard = crate::DatabaseWriterGuard::acquire(database_path.as_ref())?;
    Ok(guard
        .open_database(database_config)?
        .disable_invite(&hash, now)?)
}

pub fn set_invite_regime(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
    regime: foks_server_db::InviteRegime,
) -> Result<()> {
    let guard = crate::DatabaseWriterGuard::acquire(database_path.as_ref())?;
    guard
        .open_database(database_config)?
        .set_invite_regime(regime)?;
    Ok(())
}

pub fn list_invites(
    database_path: impl AsRef<Path>,
    database_config: foks_server_db::Config,
) -> Result<Vec<foks_server_db::InviteSnapshot>> {
    Ok(foks_server_db::ReadDatabase::open(database_path, database_config)?.invites()?)
}
