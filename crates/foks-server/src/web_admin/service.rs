//! Typed boundary between bounded transports and transactional browser authority.
use super::{AdminClock, WebAdminConfig};
use crate::{Entropy, Error, ReadDatabaseConfig, Result, WriterHandle};
use foks_server_db::{AdminMoment, ReadSnapshot, WebCredential, WebMutationAuth};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use zeroize::Zeroizing;
pub(crate) const TICKET_DOMAIN: u64 = 0xd31a_bda1_0000_0001;
pub(crate) const PENDING_DOMAIN: u64 = 0xd31a_bda1_0000_0002;
pub(crate) const SESSION_DOMAIN: u64 = 0xd31a_bda1_0000_0003;
pub(crate) const PENDING_CSRF_DOMAIN: u64 = 0xd31a_bda1_0000_0004;
pub(crate) const SESSION_CSRF_DOMAIN: u64 = 0xd31a_bda1_0000_0005;
pub(crate) const VERIFY_CSRF_DOMAIN: u64 = 0xd31a_bda1_0000_0006;
pub(crate) const NONCE_DOMAIN: u64 = 0xd31a_bda1_0000_0007;
pub(crate) fn hash(domain: u64, value: &[u8]) -> [u8; 32] {
    foks_crypto::prefixed_hash(domain, value)
}
pub(crate) struct Secret<const N: usize>(pub Zeroizing<[u8; N]>);
impl<const N: usize> std::fmt::Debug for Secret<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}
impl<const N: usize> Secret<N> {
    pub fn hex(&self) -> Zeroizing<String> {
        Zeroizing::new(hex(&*self.0))
    }
}
pub(crate) fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn unhex<const N: usize>(v: &str) -> Result<[u8; N]> {
    if v.len() != N * 2
        || !v
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Config("invalid admin identifier"));
    }
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&v[i * 2..i * 2 + 2], 16)
            .map_err(|_| Error::Config("invalid admin identifier"))?;
    }
    Ok(out)
}
pub struct WebAdminService {
    pub(crate) config: WebAdminConfig,
    pub(crate) clock: Arc<AdminClock>,
    readers: crate::read_pool::ReadPool,
    writer: WriterHandle,
    pub(crate) entropy: Arc<dyn Entropy>,
    pub(crate) ready: AtomicBool,
    writes: Arc<tokio::sync::Semaphore>,
}
impl WebAdminService {
    pub(crate) fn new(
        config: WebAdminConfig,
        clock: Arc<AdminClock>,
        read: ReadDatabaseConfig,
        writer: WriterHandle,
        entropy: Arc<dyn Entropy>,
    ) -> Result<Arc<Self>> {
        config.validate()?;
        Ok(Arc::new(Self {
            config,
            clock,
            readers: crate::read_pool::ReadPool::new(read, 2)?,
            writer,
            entropy,
            ready: AtomicBool::new(false),
            writes: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
    }
    pub(crate) fn require_ready(&self) -> Result<()> {
        if !self.ready.load(Ordering::Acquire) {
            return Err(Error::Config("admin unavailable"));
        }
        Ok(())
    }
    pub(crate) fn random<const N: usize>(&self) -> Result<Secret<N>> {
        let mut raw = Zeroizing::new([0; N]);
        self.entropy.fill(&mut *raw)?;
        Ok(Secret(raw))
    }
    pub(crate) fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut foks_server_db::Database, AdminMoment) -> foks_server_db::Result<T>
            + Send
            + 'static,
    ) -> Result<T> {
        self.require_ready()?;
        let permit = self
            .writes
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Database(foks_server_db::Error::Capacity("admin writer")))?;
        let clock = self.clock.clone();
        self.writer.call(move |db| {
            let _permit = permit;
            let now = clock.sample()?;
            Ok(f(db, now)?)
        })
    }
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&ReadSnapshot<'_>, AdminMoment) -> foks_server_db::Result<T>,
    ) -> Result<T> {
        self.require_ready()?;
        let db = self.readers.checkout()?;
        let snapshot = db.snapshot()?;
        let now = self.clock.sample()?;
        Ok(f(&snapshot, now)?)
    }
    pub(crate) fn ticket(&self, credential: WebCredential) -> Result<Zeroizing<String>> {
        let ticket = self.random::<20>()?;
        let hashed = hash(TICKET_DOMAIN, &*ticket.0);
        let id = *self.random::<16>()?.0;
        self.write(move |db, now| db.web_issue_ticket(&credential, &hashed, &id, now))?;
        Ok(Zeroizing::new(format!(
            "{}/?session={}",
            self.config.origin(),
            foks_proto::encode_base62_strict(&*ticket.0)
        )))
    }
    pub(crate) fn parse_ticket(&self, input: &str) -> Result<[u8; 32]> {
        if input.len() > 8192 {
            return Err(Error::Config("invalid admin URL"));
        }
        let url = url::Url::parse(input).map_err(|_| Error::Config("invalid admin URL"))?;
        let mut tickets = url.query_pairs().filter(|(k, _)| k == "session");
        let (_, ticket) = tickets
            .next()
            .ok_or(Error::Config("missing admin ticket"))?;
        if tickets.next().is_some() || ticket.len() != 27 {
            return Err(Error::Config("invalid admin ticket"));
        }
        let raw = Zeroizing::new(foks_proto::decode_base62_strict(&ticket)?);
        if raw.len() != 20 || foks_proto::encode_base62_strict(&raw) != ticket {
            return Err(Error::Config("invalid admin ticket"));
        }
        Ok(hash(TICKET_DOMAIN, &raw))
    }
    pub(crate) fn check(&self, input: &str, uid: &[u8; 33]) -> Result<()> {
        let ticket = self.parse_ticket(input)?;
        self.read(|db, now| db.web_check_ticket(&ticket, uid, now))
    }
    pub(crate) fn auth(cookie: &[u8; 32], csrf: &[u8; 32]) -> WebMutationAuth {
        WebMutationAuth {
            session_hash: hash(SESSION_DOMAIN, cookie),
            csrf_hash: hash(VERIFY_CSRF_DOMAIN, csrf),
        }
    }
}
