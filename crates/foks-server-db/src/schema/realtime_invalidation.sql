-- Invalidation is in the same transaction as membership changes. Update only
-- existing local Chat inboxes; remote users and nested teams need no inbox.
-- Channel configuration is immutable; creation stamps all eligible recipients.
-- Future configuration edits/deletes must invalidate or fan out transactionally.
CREATE TRIGGER rt_membership_insert AFTER INSERT ON team_members BEGIN
    UPDATE rt_user_inboxes SET reconcile_dirty=1, reconcile_after=NULL, reconcile_memberships=NULL
    WHERE uid=NEW.party_id AND app_id=1 AND (reconcile_dirty<>1 OR reconcile_after IS NOT NULL OR reconcile_memberships IS NOT NULL);
END;
CREATE TRIGGER rt_membership_delete AFTER DELETE ON team_members BEGIN
    UPDATE rt_user_inboxes SET reconcile_dirty=1, reconcile_after=NULL, reconcile_memberships=NULL
    WHERE uid=OLD.party_id AND app_id=1 AND (reconcile_dirty<>1 OR reconcile_after IS NOT NULL OR reconcile_memberships IS NOT NULL);
END;
CREATE TRIGGER rt_membership_update AFTER UPDATE ON team_members
WHEN OLD.team_id IS NOT NEW.team_id
  OR OLD.party_id IS NOT NEW.party_id
  OR OLD.scoped_host_id IS NOT NEW.scoped_host_id
  OR OLD.source_role_type IS NOT NEW.source_role_type
  OR OLD.source_visibility IS NOT NEW.source_visibility
  OR OLD.role_type IS NOT NEW.role_type
  OR OLD.visibility IS NOT NEW.visibility
BEGIN
    UPDATE rt_user_inboxes SET reconcile_dirty=1, reconcile_after=NULL, reconcile_memberships=NULL
    WHERE uid=OLD.party_id AND app_id=1 AND (reconcile_dirty<>1 OR reconcile_after IS NOT NULL OR reconcile_memberships IS NOT NULL);
    UPDATE rt_user_inboxes SET reconcile_dirty=1, reconcile_after=NULL, reconcile_memberships=NULL
    WHERE uid=NEW.party_id AND app_id=1 AND (reconcile_dirty<>1 OR reconcile_after IS NOT NULL OR reconcile_memberships IS NOT NULL);
END;
CREATE TRIGGER rt_team_access_update AFTER UPDATE ON teams
WHEN OLD.host_id IS NOT NEW.host_id OR OLD.team_kind IS NOT NEW.team_kind
BEGIN
    UPDATE rt_user_inboxes SET reconcile_dirty=1, reconcile_after=NULL, reconcile_memberships=NULL
    WHERE uid IN (SELECT party_id FROM team_members WHERE team_id=NEW.team_id)
      AND app_id=1 AND (reconcile_dirty<>1 OR reconcile_after IS NOT NULL OR reconcile_memberships IS NOT NULL);
END;
