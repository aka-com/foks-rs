use std::sync::Mutex;

/// Fixed table kinds; never derived from database contents or request identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum ExpiryKind {
    Names,
    TeamNames,
    RecoveryChallenges,
    TeamViewTokens,
    TeamViewChallenges,
    TeamAdminTokens,
    SsoSessions,
    LogSends,
    RequestReceipts,
}

impl ExpiryKind {
    pub const ALL: [Self; 9] = [
        Self::Names,
        Self::TeamNames,
        Self::RecoveryChallenges,
        Self::TeamViewTokens,
        Self::TeamViewChallenges,
        Self::TeamAdminTokens,
        Self::SsoSessions,
        Self::LogSends,
        Self::RequestReceipts,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Names => "names",
            Self::TeamNames => "team_names",
            Self::RecoveryChallenges => "recovery_challenges",
            Self::TeamViewTokens => "team_view_tokens",
            Self::TeamViewChallenges => "team_view_challenges",
            Self::TeamAdminTokens => "team_admin_tokens",
            Self::SsoSessions => "sso_sessions",
            Self::LogSends => "log_sends",
            Self::RequestReceipts => "request_receipts",
        }
    }

    fn deleted(self, report: &foks_server_db::MaintenanceReport) -> u64 {
        match self {
            Self::Names => report.reservations,
            Self::TeamNames => report.team_reservations,
            Self::RecoveryChallenges => report.challenges,
            Self::TeamViewTokens => report.team_view_tokens,
            Self::TeamViewChallenges => report.team_view_challenges,
            Self::TeamAdminTokens => report.team_admin_tokens,
            Self::SsoSessions => report.sso_sessions,
            Self::LogSends => report.log_sends,
            Self::RequestReceipts => report.receipts,
        }
    }
}

/// General maintenance only; excludes foreground opportunistic cleanup.
/// Arrays use `ExpiryKind` discriminants. Deleted rows count parents, not cascades
/// or bytes. An empty pass found no eligible rows at the sampled cleanup time;
/// it does not imply that the table was empty.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExpiryMetricsSnapshot {
    pub deleted_parent_rows: [u64; 9],
    pub empty_passes: [u64; 9],
}

#[derive(Default)]
pub(super) struct ExpiryMetrics(Mutex<ExpiryMetricsSnapshot>);

impl ExpiryMetrics {
    pub(super) fn snapshot(&self) -> ExpiryMetricsSnapshot {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn committed(&self, report: &foks_server_db::MaintenanceReport) {
        let mut snapshot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ExpiryKind::ALL {
            let index = kind as usize;
            let deleted = kind.deleted(report);
            snapshot.deleted_parent_rows[index] =
                snapshot.deleted_parent_rows[index].saturating_add(deleted);
            snapshot.empty_passes[index] =
                snapshot.empty_passes[index].saturating_add(u64::from(deleted == 0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_reports_map_every_kind_and_count_empty_passes() {
        let metrics = ExpiryMetrics::default();
        assert_eq!(metrics.snapshot(), ExpiryMetricsSnapshot::default());
        metrics.committed(&foks_server_db::MaintenanceReport {
            reservations: 1,
            team_reservations: 2,
            challenges: 3,
            team_view_tokens: 4,
            team_view_challenges: 5,
            team_admin_tokens: 6,
            sso_sessions: 7,
            log_sends: 8,
            receipts: 9,
            ..Default::default()
        });
        assert_eq!(
            metrics.snapshot().deleted_parent_rows,
            [1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert_eq!(metrics.snapshot().empty_passes, [0; 9]);
        metrics.committed(&Default::default());
        assert_eq!(
            metrics.snapshot().deleted_parent_rows,
            [1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert_eq!(metrics.snapshot().empty_passes, [1; 9]);
    }
}
