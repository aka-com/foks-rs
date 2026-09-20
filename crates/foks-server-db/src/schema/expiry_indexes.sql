-- Shared by fresh schema creation and the atomic version-45 writer upgrade.
CREATE INDEX names_reservation_expiry
    ON names(expires_at) WHERE uid IS NULL;
-- The constant NULL prefix gives the planner equality plus an expiry range,
-- rather than the older equality-only unique index, even without statistics.
CREATE INDEX team_names_reservation_expiry
    ON team_names(team_id, expires_at) WHERE team_id IS NULL;
CREATE INDEX recovery_challenges_cleanup
    ON recovery_challenges(consumed, expires_at);
CREATE INDEX team_view_tokens_expiry ON team_view_tokens(expires_at);
CREATE INDEX team_view_challenges_expiry ON team_view_challenges(expires_at);
CREATE INDEX team_admin_tokens_expiry ON team_admin_tokens(expires_at);
CREATE INDEX log_sends_created_at ON log_sends(created_at);
