import { useEffect, useRef, useState } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import type {
  InvitationAction,
  InvitationReply,
  InvitationRole,
  InvitationRow,
} from '../invitation-contract';
export function InvitationPanel({
  bridge,
  profile,
  account,
  teamAlias,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  teamAlias?: string;
  onComplete: () => Promise<void> | void;
}) {
  const [invite, setInvite] = useState('');
  const [remote, setRemote] = useState('');
  const [sourceTeam, setSourceTeam] = useState('');
  const [sourceRole, setSourceRole] = useState('admin');
  const [role, setRole] = useState('member');
  const [pin, setPin] = useState('');
  const [preview, setPreview] = useState<InvitationRow | null>(null);
  const [reply, setReply] = useState<InvitationReply | null>(null);
  const [rows, setRows] = useState<InvitationRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const identity = `${profile}/${account}/${teamAlias ?? ''}`;
  const active = useRef(identity);
  useEffect(() => {
    active.current = identity;
    setInvite('');
    setRemote('');
    setSourceTeam('');
    setPin('');
    setPreview(null);
    setReply(null);
    setRows([]);
    setBusy(false);
    setError(null);
    return () => {
      active.current = '';
    };
  }, [identity]);
  const nativeRole = (value: string): InvitationRole =>
    value === 'member'
      ? { member: { visibility: 0 } }
      : (value as 'admin' | 'owner');
  const run = async (action: InvitationAction) => {
    setBusy(true);
    setError(null);
    const secret = pin || null;
    setPin('');
    try {
      const value = await bridge.invitation(profile, account, action, secret);
      if (active.current !== identity) return;
      setReply(value);
      if (!Array.isArray(value)) {
        if (action.action === 'preview' || action.action === 'preview-remote')
          setPreview(value);
        if (value.rows) setRows(value.rows);
        if (action.action === 'inspect-remote')
          setRows((old) =>
            old.map((r) =>
              r.request_id === value.request_id ? { ...r, ...value } : r,
            ),
          );
        if (
          value.membership_verified ||
          (['approve', 'approve-remote'].includes(action.action) &&
            value.state === 'complete')
        )
          await onComplete();
      }
    } catch (e) {
      if (active.current === identity)
        setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === identity) setBusy(false);
    }
  };
  const reports = reply ? (Array.isArray(reply) ? reply : [reply]) : [];
  const requestMembership = () => {
    if (sourceTeam)
      return run(
        remote
          ? {
              action: 'accept-team-remote',
              remote_profile: remote,
              invite,
              source_team_alias: sourceTeam,
              source_role: nativeRole(sourceRole),
            }
          : {
              action: 'accept-team',
              invite,
              source_team_alias: sourceTeam,
              source_role: nativeRole(sourceRole),
            },
      );
    return run(
      remote
        ? { action: 'accept-remote', remote_profile: remote, invite }
        : { action: 'accept', invite },
    );
  };
  return (
    <section
      className="pcard"
      aria-label={
        teamAlias
          ? 'Group invitations and requests'
          : `Join a group as ${account}`
      }
    >
      <h3>
        {teamAlias ? 'Invitations & requests' : `Join a group · ${account}`}
      </h3>
      <p>
        An invitation submits a membership request. Files and chat become
        available after an administrator admits you and your client verifies the
        group keys.
      </p>
      <label>
        Other server profile, for a remote request{' '}
        <input
          value={remote}
          maxLength={128}
          disabled={busy}
          onChange={(e) => {
            setRemote(e.target.value);
            setPreview(null);
            setRows((r) =>
              r.map((x) => (x.remote ? { ...x, verified: false } : x)),
            );
          }}
        />
      </label>
      <label>
        Security key PIN, if needed{' '}
        <input
          type="password"
          autoComplete="off"
          maxLength={32}
          value={pin}
          disabled={busy}
          onChange={(e) => setPin(e.target.value)}
        />
      </label>
      {teamAlias ? (
        <>
          <button
            disabled={busy}
            onClick={() =>
              void run({ action: 'create', team_alias: teamAlias })
            }
          >
            Prepare invitation
          </button>
          <button
            disabled={busy}
            onClick={() => void run({ action: 'inbox', team_alias: teamAlias })}
          >
            Refresh requests
          </button>
          <button
            disabled={busy}
            onClick={() =>
              void run({ action: 'pending-approvals', team_alias: teamAlias })
            }
          >
            Recover approvals
          </button>
          <label>
            Role to grant{' '}
            <select
              value={role}
              disabled={busy}
              onChange={(e) => setRole(e.target.value)}
            >
              <option value="member">Member</option>
              <option value="admin">Admin · local only</option>
              <option value="owner">Owner · local only</option>
            </select>
          </label>
          <details>
            <summary>Group nesting order</summary>
            <p>
              Joining groups must have a lower signed index range than their
              destination. These changes narrow this group's range and are
              checked against existing memberships.
            </p>
            <button
              disabled={busy}
              onClick={() =>
                void run({
                  action: 'range',
                  team_alias: teamAlias,
                  raise: false,
                })
              }
            >
              Lower this group's range
            </button>
            <button
              disabled={busy}
              onClick={() =>
                void run({
                  action: 'range',
                  team_alias: teamAlias,
                  raise: true,
                })
              }
            >
              Raise this group's range
            </button>
          </details>
          {rows.map((row) => (
            <article key={row.request_id}>
              <p>
                {row.username ?? 'Unverified requester'} ·{' '}
                {row.joiner_kind ?? (row.remote ? 'remote' : 'local')}
              </p>
              {row.joiner_id && <code>{row.joiner_id}</code>}
              {row.error && !row.verified && <p>{row.error}</p>}
              {row.remote && (
                <button
                  disabled={busy || !remote}
                  onClick={() =>
                    void run({
                      action: 'inspect-remote',
                      remote_profile: remote,
                      team_alias: teamAlias,
                      request_id: row.request_id!,
                    })
                  }
                >
                  Verify on selected server
                </button>
              )}
              <button
                disabled={
                  busy ||
                  !row.verified ||
                  (!!row.remote && (!remote || role !== 'member'))
                }
                onClick={() =>
                  void run(
                    row.remote
                      ? {
                          action: 'approve-remote',
                          remote_profile: remote,
                          team_alias: teamAlias,
                          request_id: row.request_id!,
                          role: nativeRole(role),
                        }
                      : {
                          action: 'approve',
                          team_alias: teamAlias,
                          request_id: row.request_id!,
                          role: nativeRole(role),
                        },
                  )
                }
              >
                Approve membership
              </button>
              <button
                disabled={busy}
                onClick={() =>
                  void run({
                    action: 'reject',
                    team_alias: teamAlias,
                    request_id: row.request_id!,
                  })
                }
              >
                Prepare rejection
              </button>
            </article>
          ))}
        </>
      ) : (
        <>
          <label>
            Invitation{' '}
            <input
              value={invite}
              maxLength={256}
              disabled={busy}
              onChange={(e) => {
                setInvite(e.target.value);
                setPreview(null);
              }}
            />
          </label>
          <button
            disabled={busy || !invite}
            onClick={() =>
              void run(
                remote
                  ? { action: 'preview-remote', remote_profile: remote, invite }
                  : { action: 'preview', invite },
              )
            }
          >
            Preview invitation
          </button>
          {preview && (
            <div>
              <p>
                {preview.name ?? 'Group'} · <code>{preview.team_id}</code>
              </p>
              <p>
                Verified invitation host: <code>{preview.host_id}</code>
              </p>
            </div>
          )}
          <label>
            Request on behalf of a local group, optional{' '}
            <input
              value={sourceTeam}
              maxLength={128}
              disabled={busy}
              onChange={(e) => setSourceTeam(e.target.value)}
            />
          </label>
          {sourceTeam && (
            <label>
              Joining group source role{' '}
              <select
                value={sourceRole}
                onChange={(e) => setSourceRole(e.target.value)}
              >
                <option value="member">Member</option>
                <option value="admin">Admin</option>
                <option value="owner">Owner</option>
              </select>
            </label>
          )}
          <button
            disabled={busy || !preview}
            onClick={() => void requestMembership()}
          >
            Prepare membership request
          </button>
          <button
            disabled={busy}
            onClick={() =>
              void bridge
                .discoverGroups(profile, account)
                .then(onComplete)
                .catch((e) => setError(normalizeCommandError(e).message))
            }
          >
            Check local membership
          </button>
          {remote && preview?.team_id && (
            <button
              disabled={busy}
              onClick={() =>
                void run({
                  action: 'sync-remote',
                  remote_profile: remote,
                  team_id: preview.team_id!,
                  ...(sourceTeam
                    ? {
                        source_team_alias: sourceTeam,
                        source_role: nativeRole(sourceRole),
                      }
                    : {}),
                })
              }
            >
              Verify remote membership and keys
            </button>
          )}
        </>
      )}
      <button disabled={busy} onClick={() => void run({ action: 'list' })}>
        Recover invitation operations
      </button>
      {reports.map((r, i) => (
        <div key={r.operation_id ?? i} role="status">
          {r.possibly_truncated && (
            <p>
              More requests may exist. The server’s time-based pagination cannot
              safely skip a full timestamp group.
            </p>
          )}
          {r.state && (
            <p>
              {r.state}
              {r.delivery_acknowledged
                ? ' · Request delivered; membership still needs approval.'
                : ''}
            </p>
          )}
          {r.membership_verified && <p>Membership and group keys verified.</p>}
          {r.invite && (
            <>
              <input
                readOnly
                value={r.invite}
                aria-label="Shareable invitation"
              />
              <button
                onClick={() =>
                  void bridge
                    .copyText(r.invite!)
                    .catch((e) => setError(normalizeCommandError(e).message))
                }
              >
                Copy invitation
              </button>
            </>
          )}
          {r.operation_id && !r.request_id && (
            <>
              <code>{r.operation_id}</code>
              <button
                disabled={busy}
                onClick={() =>
                  void run(
                    r.remote
                      ? {
                          action: 'status-remote',
                          remote_profile: r.remote_profile ?? remote,
                          operation_id: r.operation_id!,
                        }
                      : { action: 'status', operation_id: r.operation_id! },
                  )
                }
              >
                Check operation
              </button>
              {r.state === 'prepared' && (
                <>
                  <button
                    disabled={busy}
                    onClick={() =>
                      void run(
                        r.remote
                          ? {
                              action: 'attempt-remote',
                              remote_profile: r.remote_profile ?? remote,
                              operation_id: r.operation_id!,
                            }
                          : {
                              action: 'attempt',
                              operation_id: r.operation_id!,
                            },
                      )
                    }
                  >
                    Submit prepared operation
                  </button>
                  <button
                    disabled={busy}
                    onClick={() =>
                      void run({
                        action: 'cancel',
                        operation_id: r.operation_id!,
                      })
                    }
                  >
                    Cancel preparation
                  </button>
                </>
              )}
              {r.state === 'submission-unknown' && (
                <p>
                  The reply was lost. Checking will not send a duplicate
                  request.
                </p>
              )}
            </>
          )}
          {r.request_id && teamAlias && r.role && r.state !== 'complete' && (
            <button
              disabled={busy}
              onClick={() =>
                void run(
                  r.remote
                    ? {
                        action: 'approve-remote',
                        remote_profile: r.source_profile!,
                        team_alias: teamAlias,
                        request_id: r.request_id!,
                        role:
                          r.role!.kind === 1
                            ? { member: { visibility: r.role!.visibility } }
                            : r.role!.kind === 2
                              ? 'admin'
                              : 'owner',
                      }
                    : {
                        action: 'approve',
                        team_alias: teamAlias,
                        request_id: r.request_id!,
                        role:
                          r.role!.kind === 1
                            ? { member: { visibility: r.role!.visibility } }
                            : r.role!.kind === 2
                              ? 'admin'
                              : 'owner',
                      },
                )
              }
            >
              Resume original approval
            </button>
          )}
        </div>
      ))}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
