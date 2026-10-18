use super::*;

// A new native table requires a deliberate policy review, never implicit acceptance.
const TABLES:&str="admin_audit bad_passphrase_attempts capability_key_generations chunk_object_attempts chunk_object_totals chunk_objects chunk_storage_binding chunk_storage_revision devices federation_team_view_permissions federation_user_view_permissions generic_chain_heads generic_chain_links generic_tree_locations host_admin_grants host_key_generations host_metadata host_rotation_operations hostchain_links issued_certificates kv_directories kv_directory_heads kv_dirent_current kv_dirent_heads kv_dirents kv_file_chunks kv_file_uploads kv_locks kv_namespaces kv_nodes kv_roots kv_upload_maintenance log_send_blocks log_send_files log_sends merkle_back_pointers merkle_leaves merkle_nodes merkle_root_heads merkle_roots metadata_read_dependency names parcels passphrase_boxes passphrase_login_challenges passphrase_salts recovery_challenges request_receipts rt_channel_sets rt_channels rt_messages rt_user_channels rt_user_inboxes seed_chain_boxes services shared_keys signup_invite_redemptions signup_invites signup_policy sso_access sso_binding_receipts sso_identity_challenges sso_migration_cohort sso_policy sso_sessions subchain_tree_location_seeds team_admin_tokens team_chain_heads team_chain_links team_invitation_certificates team_local_join_requests team_local_view_permissions team_members team_names team_parcels team_remote_join_requests team_remote_member_view_tokens team_removal_boxes team_removal_proofs team_seed_chain_boxes team_shared_keys team_tree_locations team_view_challenges team_view_tokens teams tree_locations user_chain_heads user_chain_links user_local_view_permissions user_name_history users waitlist_count waitlist_entries web_admin_nonces web_admin_sessions web_login_confirmations web_login_tickets yubi_management_keys yubi_pq_hints yubi_subkey_boxes";
pub(super) const EMPTY: &[&str] = &[
    "teams",
    "team_names",
    "team_members",
    "team_chain_heads",
    "team_chain_links",
    "team_tree_locations",
    "team_shared_keys",
    "team_parcels",
    "team_seed_chain_boxes",
    "team_admin_tokens",
    "team_invitation_certificates",
    "team_local_join_requests",
    "team_local_view_permissions",
    "team_remote_join_requests",
    "team_remote_member_view_tokens",
    "team_removal_boxes",
    "team_removal_proofs",
    "team_view_challenges",
    "team_view_tokens",
    "sso_access",
    "sso_binding_receipts",
    "sso_identity_challenges",
    "sso_migration_cohort",
    "sso_policy",
    "sso_sessions",
    "federation_team_view_permissions",
    "federation_user_view_permissions",
    "rt_channel_sets",
    "rt_channels",
    "rt_messages",
    "rt_user_channels",
    "rt_user_inboxes",
];
pub(super) fn fingerprint() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"foks-native-personal-list-import-v1\0");
    hash.update(TABLES);
    hash.update(scan::SCHEMA_HASH);
    for (name, digest) in HOSTED_SMALL_V1.source_pins() {
        hash.update(name);
        hash.update([0]);
        hash.update(digest);
        hash.update([0]);
    }
    for name in EMPTY {
        hash.update(name);
        hash.update([0]);
    }
    hash.update(b"generic=2/local-user;namespace=1/local-user/local-host;view-target=user;external=0;gen-each<=4096;hostchain<=4096/1048576;probe<=1048576;finite-real/bit-identical;UTF8;source=finalized-small-v1;fresh-incarnation;conditional-native-authority;existing-host-keys");
    hash.finalize().into()
}
fn none(db: &scan::Database, sql: &str) -> Result<()> {
    db.control().statement()?;
    if db
        .connection()
        .query_row(sql, [], |r| r.get::<_, bool>(0))?
    {
        return Err(unsupported("personal import unsupported data"));
    }
    Ok(())
}
pub(super) fn verify(db: &scan::Database) -> Result<()> {
    if !TABLES.split_whitespace().eq(HOSTED_SMALL_V1
        .descriptor(Component::Foks)
        .tables()
        .map(|t| t.name()))
    {
        return Err(unsupported("personal policy table inventory"));
    }
    for table in EMPTY {
        none(
            db,
            &format!("SELECT EXISTS(SELECT 1 FROM main.{})", copy::quote(table)),
        )?;
    }
    for table in [
        "generic_chain_heads",
        "generic_chain_links",
        "generic_tree_locations",
    ] {
        none(db,&format!("SELECT EXISTS(SELECT 1 FROM main.{} g WHERE g.chain_type!=2 OR NOT EXISTS(SELECT 1 FROM main.users u WHERE u.uid=g.entity_id))",copy::quote(table)))?;
    }
    none(db,"SELECT EXISTS(SELECT 1 FROM main.subchain_tree_location_seeds s WHERE NOT EXISTS(SELECT 1 FROM main.users u WHERE u.uid=s.entity_id))")?;
    none(db,"SELECT EXISTS(SELECT 1 FROM main.kv_namespaces n WHERE n.party_kind!=1 OR NOT EXISTS(SELECT 1 FROM main.users u WHERE u.uid=n.namespace_id) OR NOT EXISTS(SELECT 1 FROM main.host_metadata h WHERE h.singleton=1 AND h.host_id=n.host_id))")?;
    for (table, column) in [
        ("users", "uid"),
        ("user_local_view_permissions", "target_id"),
    ] {
        db.control().statement()?;
        let sql=format!("SELECT EXISTS(SELECT 1 FROM main.{} WHERE typeof({column})!='blob' OR length({column})!=33 OR substr({column},1,1)!=?1)",copy::quote(table),column=copy::quote(column));
        if db
            .connection()
            .query_row(&sql, [&[foks_proto::ENTITY_USER][..]], |r| {
                r.get::<_, bool>(0)
            })?
        {
            return Err(unsupported("personal user entity type"));
        }
    }
    none(
        db,
        "SELECT EXISTS(SELECT 1 FROM main.chunk_storage_binding)",
    )?;
    none(db,"SELECT EXISTS(SELECT 1 FROM main.kv_file_chunks WHERE external_object_id IS NOT NULL OR external_digest IS NOT NULL OR external_encoded_bytes IS NOT NULL)")?;
    for table in ["host_key_generations", "capability_key_generations"] {
        db.control().statement()?;
        let count: i64 = db.connection().query_row(
            &format!(
                "SELECT count(*) FROM (SELECT 1 FROM main.{} LIMIT 4097)",
                copy::quote(table)
            ),
            [],
            |r| r.get(0),
        )?;
        if count > 4096 {
            return Err(unsupported("personal key generation history cap"));
        }
    }
    db.control().check()
}
