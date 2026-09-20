//! Bounded expiry statements. Selectors are shared with work-counter tests.
//! Index choice remains SQLite's; no recurring statistics or discovery queries.
#[derive(Clone, Copy)]
pub(crate) struct ExpiryStatement {
    #[cfg(test)]
    pub table: &'static str,
    #[cfg(test)]
    pub select: &'static str,
    pub delete: &'static str,
}

macro_rules! statement {
    ($name:ident, $table:literal, $select:literal) => {
        pub(crate) const $name: ExpiryStatement = ExpiryStatement {
            #[cfg(test)]
            table: $table,
            #[cfg(test)]
            select: $select,
            delete: concat!("DELETE FROM ", $table, " WHERE rowid IN (", $select, ")"),
        };
    };
}

// Every valid session references a policy host, including disabled policies.
statement!(SSO_SESSIONS, "sso_sessions", "SELECT rowid FROM sso_sessions WHERE host IN (SELECT host FROM sso_policy) AND expires_at_ms <= ?1 LIMIT 128");
statement!(
    RESERVATIONS,
    "names",
    "SELECT rowid FROM names WHERE uid IS NULL AND expires_at <= ?1 LIMIT 128"
);
statement!(
    TEAM_RESERVATIONS,
    "team_names",
    "SELECT rowid FROM team_names WHERE team_id IS NULL AND expires_at <= ?1 LIMIT 128"
);
statement!(
    RECEIPTS,
    "request_receipts",
    "SELECT rowid FROM request_receipts WHERE expires_at <= ?1 LIMIT 128"
);
// Disjoint branches stream by expiry and rowid from (consumed, expires_at).
// One shared limit applies even when a consumed row is also expired.
statement!(
    CHALLENGES,
    "recovery_challenges",
    "SELECT candidate_rowid FROM (
    SELECT rowid AS candidate_rowid, expires_at FROM recovery_challenges
    WHERE consumed = 0 AND expires_at <= ?1
    UNION ALL
    SELECT rowid AS candidate_rowid, expires_at FROM recovery_challenges
    WHERE consumed = 1
    ORDER BY expires_at, candidate_rowid
    LIMIT 128
)"
);
statement!(
    TEAM_VIEW_TOKENS,
    "team_view_tokens",
    "SELECT rowid FROM team_view_tokens WHERE expires_at <= ?1 LIMIT 128"
);
statement!(
    TEAM_VIEW_CHALLENGES,
    "team_view_challenges",
    "SELECT rowid FROM team_view_challenges WHERE expires_at <= ?1 LIMIT 128"
);
statement!(
    TEAM_ADMIN_TOKENS,
    "team_admin_tokens",
    "SELECT rowid FROM team_admin_tokens WHERE expires_at <= ?1 LIMIT 128"
);
statement!(
    LOG_SENDS,
    "log_sends",
    "SELECT rowid FROM log_sends WHERE created_at <= ?1 LIMIT 128"
);

#[cfg(test)]
pub(crate) const ALL: [ExpiryStatement; 9] = [
    SSO_SESSIONS,
    RESERVATIONS,
    TEAM_RESERVATIONS,
    RECEIPTS,
    CHALLENGES,
    TEAM_VIEW_TOKENS,
    TEAM_VIEW_CHALLENGES,
    TEAM_ADMIN_TOKENS,
    LOG_SENDS,
];
