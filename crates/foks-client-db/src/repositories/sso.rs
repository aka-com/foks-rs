//! Public OAuth progress. Session IDs, tokens, provider claims and verifier live only in protected storage.
use crate::{sqlite_integer, Error, HardStateStore, Result};
use rusqlite::{params, OptionalExtension};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SsoFlowState {
    Prepared = 0,
    AwaitingBrowser = 1,
    Ready = 2,
    Binding = 3,
    Complete = 4,
    Cancelled = 5,
    Expired = 6,
    Unknown = 7,
    Rejected = 8,
    Denied = 9,
}
impl SsoFlowState {
    fn parse(n: u8) -> rusqlite::Result<Self> {
        use SsoFlowState::*;
        Ok(match n {
            0 => Prepared,
            1 => AwaitingBrowser,
            2 => Ready,
            3 => Binding,
            4 => Complete,
            5 => Cancelled,
            6 => Expired,
            7 => Unknown,
            8 => Rejected,
            9 => Denied,
            _ => return Err(rusqlite::Error::InvalidQuery),
        })
    }
    pub fn permits(self, next: Self) -> bool {
        use SsoFlowState::*;
        matches!(
            (self, next),
            (Prepared, AwaitingBrowser | Cancelled | Expired)
                | (AwaitingBrowser, Ready | Cancelled | Expired | Denied)
                | (Ready, Binding | Cancelled | Expired)
                | (Binding, Complete | Unknown | Rejected)
                | (Unknown, Complete | Rejected)
        )
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsoFlow {
    pub id: [u8; 16],
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub device: Vec<u8>,
    pub purpose: foks_proto::SsoPurpose,
    pub state: SsoFlowState,
    pub material_hash: [u8; 32],
    pub config_hash: [u8; 32],
    pub expires_at_ms: u64,
    pub commitment: Option<[u8; 32]>,
    pub final_operation: Option<[u8; 16]>,
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SsoFlow> {
    Ok(SsoFlow {
        id: r.get(0)?,
        host: r.get(1)?,
        uid: r.get(2)?,
        device: r.get(3)?,
        purpose: foks_proto::SsoPurpose::from_code(r.get(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        commitment: r.get(10)?,
        state: SsoFlowState::parse(r.get(5)?)?,
        material_hash: r.get(6)?,
        config_hash: r.get(7)?,
        expires_at_ms: r.get::<_, i64>(8)? as u64,
        final_operation: r.get(9)?,
    })
}
const SELECT: &str = "SELECT operation_id,host_id,uid,device_id,purpose,state,material_hash,config_hash,expires_at,final_operation,commitment FROM sso_flows";
impl HardStateStore {
    pub fn sso_flow(&self, id: &[u8; 16]) -> Result<Option<SsoFlow>> {
        Ok(self
            .connection
            .query_row(
                &format!("{SELECT} WHERE operation_id=?1"),
                [id.as_slice()],
                row,
            )
            .optional()?)
    }
    pub fn sso_flows(&self, host: &[u8], uid: &[u8]) -> Result<Vec<SsoFlow>> {
        let mut stmt = self.connection.prepare(&format!(
            "{SELECT} WHERE host_id=?1 AND uid=?2 ORDER BY operation_id LIMIT 4097"
        ))?;
        let rows = stmt
            .query_map(params![host, uid], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn sso_record_with_protected_payload<E: From<Error>>(
        &mut self,
        flow: &SsoFlow,
        now_ms: u64,
        persist: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        if flow.state != SsoFlowState::Prepared
            || flow.final_operation.is_some()
            || flow.commitment.is_some()
            || flow.expires_at_ms <= now_ms
        {
            return Err(Error::SsoState("invalid new flow").into());
        }
        let now = sqlite_integer("OAuth creation time", now_ms)?;
        let expiry = sqlite_integer("OAuth expiry", flow.expires_at_ms)?;
        let tx = self.write_transaction()?;
        let (active,total):(i64,i64)=tx.query_row("SELECT coalesce(sum(CASE WHEN state IN (0,1,2,3,7) AND (expires_at>?3 OR (state IN (3,7) AND final_operation IS NOT NULL)) THEN 1 ELSE 0 END),0),count(*) FROM sso_flows WHERE host_id=?1 AND uid=?2",params![flow.host,flow.uid,now],|r|Ok((r.get(0)?,r.get(1)?))).map_err(Error::from)?;
        if active >= 4 || total >= 4096 {
            return Err(Error::SsoState("flow capacity exhausted").into());
        }
        tx.execute("INSERT INTO sso_flows(operation_id,host_id,uid,device_id,purpose,state,material_hash,config_hash,expires_at,final_operation) VALUES (?1,?2,?3,?4,?5,0,?6,?7,?8,NULL)",params![flow.id.as_slice(),flow.host,flow.uid,flow.device,flow.purpose as u8,flow.material_hash.as_slice(),flow.config_hash.as_slice(),expiry]).map_err(Error::from)?;
        persist()?;
        tx.commit().map_err(Error::from)?;
        Ok(())
    }
    /// Write once, before delivery. Nonsecret evidence survives protected-payload erasure.
    pub fn sso_set_commitment(&mut self, id: &[u8; 16], commitment: &[u8; 32]) -> Result<()> {
        let tx = self.write_transaction()?;
        let changed=tx.execute("UPDATE sso_flows SET commitment=?2 WHERE operation_id=?1 AND state=2 AND (commitment IS NULL OR commitment=?2)",params![id,commitment])?;
        if changed != 1 {
            return Err(Error::SsoState(
                "binding commitment changed or flow is not ready",
            ));
        }
        tx.commit()?;
        Ok(())
    }
    pub fn sso_transition(
        &mut self,
        id: &[u8; 16],
        from: SsoFlowState,
        to: SsoFlowState,
        final_operation: Option<&[u8; 16]>,
    ) -> Result<()> {
        if !from.permits(to) || (final_operation.is_some() && to != SsoFlowState::Binding) {
            return Err(Error::SsoState("transition is not permitted"));
        }
        let tx = self.write_transaction()?;
        if let Some(op) = final_operation {
            let matches:i64=tx.query_row("SELECT count(*) FROM mutation_operations m JOIN sso_flows s ON s.operation_id=?1 WHERE m.operation_id=?2 AND m.operation_kind=1 AND m.host_id=s.host_id AND m.subject_id=s.uid AND m.scope_id=s.device_id",params![id.as_slice(),op.as_slice()],|r|r.get(0))?;
            if matches != 1 {
                return Err(Error::SsoState("signup operation does not match flow"));
            }
        }
        let count=tx.execute("UPDATE sso_flows SET state=?3,final_operation=coalesce(?4,final_operation) WHERE operation_id=?1 AND state=?2",params![id.as_slice(),from as u8,to as u8,final_operation.map(|id|id.as_slice())])?;
        if count != 1 {
            return Err(Error::SsoState("flow changed concurrently"));
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flow_claims_are_durable_bounded_and_signup_linkage_is_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hard.sqlite3");
        let mut db = HardStateStore::open(&path).unwrap();
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            )),
        )
        .unwrap();
        db.accept_verified_host(&verified.snapshot).unwrap();
        let mut flow = SsoFlow {
            id: [1; 16],
            host: verified.snapshot.host_id().to_vec(),
            uid: vec![1; 33],
            device: vec![4; 33],
            purpose: foks_proto::SsoPurpose::Signup,
            commitment: None,
            state: SsoFlowState::Prepared,
            material_hash: [5; 32],
            config_hash: [6; 32],
            expires_at_ms: 600_000,
            final_operation: None,
        };
        let before = db.metadata().unwrap();
        assert!(db
            .sso_record_with_protected_payload::<Error>(&flow, 0, || Err(Error::SsoState(
                "store unavailable"
            )))
            .is_err());
        assert!(db.sso_flow(&flow.id).unwrap().is_none());
        db.sso_record_with_protected_payload::<Error>(&flow, 0, || Ok(()))
            .unwrap();
        assert!(db.metadata().unwrap().revision > before.revision);
        db.sso_transition(
            &flow.id,
            SsoFlowState::Prepared,
            SsoFlowState::AwaitingBrowser,
            None,
        )
        .unwrap();
        assert!(db
            .sso_transition(
                &flow.id,
                SsoFlowState::Prepared,
                SsoFlowState::AwaitingBrowser,
                None
            )
            .is_err());
        db.sso_transition(
            &flow.id,
            SsoFlowState::AwaitingBrowser,
            SsoFlowState::Ready,
            None,
        )
        .unwrap();
        let mut op = crate::MutationOperation {
            operation_id: [10; 16],
            kind: crate::MutationKind::Signup,
            host_id: flow.host.clone(),
            scope_id: flow.device.clone(),
            subject_id: vec![9; 33],
            expected_version: None,
            request_hash: [11; 32],
            material_ref: vec![12],
            material_hash: [13; 32],
            state: crate::MutationState::Prepared,
            attempt_count: 0,
            created_at: 1,
            updated_at: 1,
        };
        assert!(db.record_sso_signup(&op, &flow.id).is_err());
        assert!(db.mutation(&op.operation_id).unwrap().is_none());
        assert_eq!(
            db.sso_flow(&flow.id).unwrap().unwrap().state,
            SsoFlowState::Ready
        );
        op.subject_id = flow.uid.clone();
        db.sso_set_commitment(&flow.id, &[99; 32]).unwrap();
        db.record_sso_signup(&op, &flow.id).unwrap();
        assert_eq!(
            db.sso_flow(&flow.id).unwrap().unwrap().final_operation,
            Some(op.operation_id)
        );
        assert!(db
            .sso_transition(
                &flow.id,
                SsoFlowState::Binding,
                SsoFlowState::Cancelled,
                None
            )
            .is_err());
        for i in 2..=4 {
            flow.id = [i; 16];
            db.sso_record_with_protected_payload::<Error>(&flow, 0, || Ok(()))
                .unwrap();
        }
        flow.id = [5; 16];
        assert!(db
            .sso_record_with_protected_payload::<Error>(&flow, 0, || panic!(
                "full pool must not stage secrets"
            ))
            .is_err());
        // Expired browser flows don't consume active slots; ambiguous final mutations do.
        db.sso_record_with_protected_payload::<Error>(
            &SsoFlow {
                expires_at_ms: 1_200_001,
                ..flow.clone()
            },
            600_001,
            || Ok(()),
        )
        .unwrap();
        drop(db);
        let db = HardStateStore::open(&path).unwrap();
        assert_eq!(
            db.sso_flow(&[1; 16]).unwrap().unwrap().state,
            SsoFlowState::Binding
        );
    }
}
