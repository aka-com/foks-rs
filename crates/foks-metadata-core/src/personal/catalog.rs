//! Fixed scalar catalog; the same text executes in native SQLite and celld.
use crate::{
    catalog::{Kind, Spec},
    Statement as S, MAX_BLOB_BYTES,
};
pub(crate) fn spec(statement: S) -> Spec {
    use Kind::{Blob as B, Integer as I, NullableBlob as N};
    let (sql,parameters,columns):(_, &'static [_], &'static [_])=match statement {
        S::PersonalIssuedCredential => ("SELECT c.uid,c.credential_id,d.device_id,CAST(d.role_type AS TEXT),CAST(d.visibility AS TEXT) FROM issued_certificates c JOIN devices d ON d.device_id=c.device_id AND d.uid=c.uid WHERE c.serial=?1 AND c.exact_certificate=?2 AND c.not_before<=CAST(?3 AS INTEGER) AND c.not_after>CAST(?3 AS INTEGER) AND d.active=1 AND (c.credential_id=d.device_id OR c.credential_id=d.subkey_id)",&[B,B,I],&[B,B,B,I,I]),
        // Native admission deliberately does not require the historical issuance
        // owner to remain the current device/subkey owner. See native_list.
        S::ListIssuedCredential => ("SELECT c.uid,c.credential_id,CAST(d.role_type AS TEXT),CAST(d.visibility AS TEXT),CAST(c.not_after AS TEXT) FROM issued_certificates c JOIN devices d ON d.device_id=c.device_id AND d.uid=c.uid WHERE c.serial=?1 AND c.exact_certificate=?2 AND c.not_before<=CAST(?3 AS INTEGER) AND c.not_after>CAST(?3 AS INTEGER) AND d.active=1",&[B,B,I],&[B,B,I,I,I]),
        S::ListCurrentOwner => ("SELECT device_id FROM devices WHERE uid=?1 AND active=1 AND (device_id=?2 OR subkey_id=?2) ORDER BY device_id LIMIT 1",&[B,B],&[B]),
        S::PersonalCurrentCredential => ("SELECT CAST(role_type AS TEXT),CAST(visibility AS TEXT) FROM devices WHERE device_id=?1 AND uid=?2 AND active=1 AND (device_id=?3 OR subkey_id=?3)",&[B,B,B],&[I,I]),
        S::PersonalAnchor => ("SELECT '1' FROM user_chain_heads h JOIN tree_locations t ON t.uid=h.uid AND t.seqno=h.seqno JOIN merkle_root_heads rh ON rh.singleton=1 JOIN merkle_roots r ON r.epoch=rh.epoch WHERE h.uid=?1",&[B],&[I]),
        // The unique expression index bounds the entire persisted host policy to
        // one row, including disabled/blocked policies and mismatched-host corruption.
        S::PersonalSsoPolicy => ("SELECT '1' FROM sso_policy INDEXED BY sso_single_host_policy WHERE (1)=1 LIMIT 1",&[],&[I]),
        S::PersonalRoleGeneration => ("SELECT CAST(generation AS TEXT) FROM shared_keys WHERE uid=?1 AND role_type=CAST(?2 AS INTEGER) AND visibility=CAST(?3 AS INTEGER) ORDER BY generation DESC LIMIT 1",&[B,I,I],&[I]),
        S::PersonalRoot => ("SELECT CAST(root_version AS TEXT),CAST(length(exact_root) AS TEXT),CASE WHEN length(exact_root)<=4096 THEN exact_root ELSE NULL END FROM kv_roots WHERE uid=?1",&[B],&[I,I,N]),
        S::PersonalDirectory => ("SELECT CAST(d.version AS TEXT),CAST(length(d.exact_directory) AS TEXT),CASE WHEN length(d.exact_directory)<=4096 THEN d.exact_directory ELSE NULL END FROM kv_directory_heads h JOIN kv_directories d ON d.uid=h.uid AND d.directory_id=h.directory_id AND d.version=h.version WHERE h.uid=?1 AND h.directory_id=?2",&[B,B],&[I,I,N]),
        S::PersonalSmallNode => ("SELECT CAST(node_type AS TEXT),CAST(length(exact_node) AS TEXT),CASE WHEN length(exact_node)<=4096 THEN exact_node ELSE NULL END FROM kv_nodes WHERE uid=?1 AND node_id=?2",&[B,B],&[I,I,N]),
        S::PublicUsage => ("SELECT CAST(usage_small_count AS TEXT),CAST(usage_small_bytes AS TEXT),CAST(usage_large_count AS TEXT),CAST(usage_large_bytes AS TEXT),CAST(usage_large_chunks AS TEXT),CAST(usage_unvalidated_small_count AS TEXT) FROM kv_namespaces WHERE namespace_id=?1",&[B],&[I,I,I,I,I,I]),
        _=>unreachable!("personal statements explicitly delegated"),
    };
    Spec {
        id: statement as u32,
        sql,
        parameters,
        columns,
        write: false,
        maximum_blob_bytes: MAX_BLOB_BYTES,
    }
}
