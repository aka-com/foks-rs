//! Durable ciphertext service. Authorization is evaluated on the same SQLite
//! transaction/snapshot as the operation, never from a cached inbox membership.
use crate::{error::sql_integer, Database, Error, ReadSnapshot, Result};
use foks_proto::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

mod limits;
pub use limits::RealtimeLimits;
use RealtimeLimits as Limits;
const MIN_ROLE: Role = Role::member(-0x4000);

/// Identity authenticated by the server. The expiry is the actual presented
/// certificate's expiry, so a queued write cannot outlive that credential.
#[derive(Clone)]
pub struct RealtimeActor {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub credential: Vec<u8>,
    pub certificate_expires_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealtimeWakeTarget {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub app: RtAppId,
}
pub struct RealtimeCommit<T> {
    pub value: T,
    pub wake: Vec<RealtimeWakeTarget>,
}

fn proto<T>(value: foks_proto::Result<T>) -> Result<T> {
    value.map_err(|_| Error::Invalid("realtime wire value"))
}
mod channels;
mod inbox;
mod messages;
mod policy;
use channels::{channel, project_channel};
use inbox::stamp;
use policy::*;

#[cfg(test)]
mod tests;

/// Durable work state, independent of the wire inbox version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeReconcileState {
    Missing,
    Dirty,
    Incomplete,
    Clean,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RealtimeInboxState {
    pub version: u64,
    pub reconciliation: RealtimeReconcileState,
}
impl RealtimeInboxState {
    pub fn needs_reconciliation(self) -> bool {
        self.reconciliation != RealtimeReconcileState::Clean
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeReconcileOutcome {
    AlreadyClean,
    PageComplete,
    PageIncomplete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RealtimeReconcileReport {
    pub outcome: RealtimeReconcileOutcome,
    pub restarted: bool,
    /// Candidates returned by the bounded query, including its lookahead row.
    pub candidates: usize,
    pub accessibility_changes: usize,
}
