import { useDeviceCache } from '../device-cache';
import { useQueryRepository } from '../query-hooks';
import { useTabSheetState } from '../navigation-guard';
import { useEffect, useRef, useState } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useSheetGuard } from '../navigation-guard';
import type {
  InvitationAction,
  InvitationReply,
  InvitationRole,
  InvitationRow,
} from '../invitation-contract';
import {
  Button,
  CopyBox,
  Icon,
  Inset,
  InsetRow,
  PanelSheet,
  SectionLabel,
} from './index';
import type { PanelPresentation } from './index';
export const INVITATION_ACTIVITY = 'foks:invitation-activity';

export function InvitationPanel({
  bridge,
  profile,
  account,
  teamAlias,
  teamId,
  presentation,
  recover = false,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  teamAlias?: string;
  teamId?: string;
  /**
   * Render the join flow as a modal sheet instead of a card: the form in the
   * body and its actions in the footer.
   */
  presentation?: PanelPresentation;
  recover?: boolean;
  onComplete: () => Promise<void> | void;
}) {
  const devices = useDeviceCache();
  const queries = useQueryRepository(bridge, devices?.repository);
  const [invite, setInvite] = useTabSheetState('invitation.invite', '');
  const [remote, setRemote] = useTabSheetState('invitation.remote', '');
  const [sourceTeam, setSourceTeam] = useTabSheetState(
    'invitation.sourceTeam',
    '',
  );
  const [sourceRole, setSourceRole] = useTabSheetState(
    'invitation.sourceRole',
    'admin',
  );
  const [role, setRole] = useTabSheetState('invitation.role', 'member');
  const [pin, setPin] = useState('');
  const [preview, setPreview] = useState<InvitationRow | null>(null);
  const [reply, setReply] = useState<InvitationReply | null>(null);
  const [rows, setRows] = useState<InvitationRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [resumable, setResumable] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const identity = `${profile}/${account}/${teamAlias ?? ''}`;
  const active = useRef(identity);
  const priorIdentity = useRef(identity);
  useEffect(() => {
    active.current = identity;
    if (priorIdentity.current !== identity) {
      setInvite('');
      setRemote('');
      setSourceTeam('');
    }
    priorIdentity.current = identity;
    setPin('');
    setPreview(null);
    setReply(null);
    setRows([]);
    setBusy(false);
    setError(null);
    return () => {
      active.current = '';
    };
  }, [identity, setInvite, setRemote, setSourceTeam]);
  useEffect(() => {
    if (!recover || !teamAlias) return;
    let live = true;
    setBusy(true);
    void Promise.all([
      bridge.invitation(profile, account, { action: 'list' }, null),
      bridge.invitation(
        profile,
        account,
        {
          action: 'pending-approvals',
          team_alias: teamAlias,
        },
        null,
      ),
    ])
      .then((values) => {
        if (live)
          setReply(
            values.flatMap((value, index) => {
              const rows = Array.isArray(value)
                ? value
                : (value.rows ?? [value]);
              return index === 0 && teamId
                ? rows.filter((row) => row.team_id === teamId)
                : rows;
            }),
          );
      })
      .catch((failure: unknown) => {
        if (live) setError(normalizeCommandError(failure).message);
      })
      .finally(() => {
        if (live) setBusy(false);
      });
    return () => {
      live = false;
    };
  }, [bridge, profile, account, teamAlias, teamId, recover]);
  const nativeRole = (value: string): InvitationRole =>
    value === 'member'
      ? { member: { visibility: 0 } }
      : (value as 'admin' | 'owner');
  const run = async (action: InvitationAction) => {
    setBusy(true);
    // These native actions retain their prepared operation or approval record.
    setResumable(
      ['create', 'approve', 'reject', 'attempt', 'status'].includes(
        action.action,
      ),
    );
    setError(null);
    const secret = pin || null;
    setPin('');
    try {
      const value = await bridge.invitation(profile, account, action, secret);
      if (
        !Array.isArray(value) &&
        (value.membership_verified ||
          (['approve', 'approve-remote'].includes(action.action) &&
            value.state === 'complete'))
      )
        await onComplete();
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
      }
    } catch (e) {
      if (active.current === identity)
        setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === identity) setBusy(false);
      if (
        ![
          'preview',
          'preview-remote',
          'inbox',
          'pending-approvals',
          'list',
          'inspect-remote',
        ].includes(action.action) &&
        !(action.action === 'range' && !action.raise)
      ) {
        // Invalidate even when the recovery banner is currently unmounted.
        queries.invalidate(['invitation-recovery', profile, account]);
        window.dispatchEvent(
          new window.CustomEvent(INVITATION_ACTIVITY, {
            detail: { profile, account },
          }),
        );
      }
    }
  };
  // Local mutations are listed durably by the agent and recovered from the
  // group banner. Other calls still require their reply before navigation.
  useSheetGuard(
    busy
      ? resumable
        ? null
        : { verdict: 'refuse', reason: 'Wait for the invitation to finish.' }
      : invite || remote || sourceTeam || pin
        ? {
            verdict: 'prompt',
            title: 'Discard invitation details?',
            body: 'What is typed here has not been sent to the server.',
            confirm: 'Discard',
            onConfirm: () => {
              setInvite('');
              setRemote('');
              setSourceTeam('');
              setPin('');
              setPreview(null);
              presentation?.onClose();
            },
          }
        : null,
    !busy && !pin,
  );
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
  const copy = (text: string) =>
    void bridge
      .copyText(text)
      .catch((e) => setError(normalizeCommandError(e).message));
  const remoteRow = (
    <InsetRow label="Server profile (for teams on another server)">
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
    </InsetRow>
  );
  const pinRow = (
    <InsetRow label="Security key PIN (enrolled keys only)">
      <input
        type="password"
        autoComplete="off"
        maxLength={32}
        value={pin}
        disabled={busy}
        onChange={(e) => setPin(e.target.value)}
      />
    </InsetRow>
  );
  const recoverButton = (
    <Button disabled={busy} onClick={() => void run({ action: 'list' })}>
      {teamAlias ? 'Show pending operations' : 'Show pending requests'}
    </Button>
  );
  const inviteRow = (
    <InsetRow label="Invitation">
      <input
        value={invite}
        maxLength={256}
        disabled={busy}
        onChange={(e) => {
          setInvite(e.target.value);
          setPreview(null);
        }}
      />
    </InsetRow>
  );
  const sourceTeamRow = (
    <InsetRow label="Requesting team (to add a team you administer instead of yourself)">
      <input
        value={sourceTeam}
        maxLength={128}
        disabled={busy}
        onChange={(e) => setSourceTeam(e.target.value)}
      />
    </InsetRow>
  );
  const sourceRoleRow = sourceTeam ? (
    <InsetRow label="Requesting team role">
      <select
        value={sourceRole}
        disabled={busy}
        onChange={(e) => setSourceRole(e.target.value)}
      >
        <option value="member">Member</option>
        <option value="admin">Admin</option>
        <option value="owner">Owner</option>
      </select>
    </InsetRow>
  ) : null;
  const previewBox = preview ? (
    <div className="op">
      <p>
        {preview.name ?? 'Team'} · <code>{preview.team_id}</code>
      </p>
      <p>
        Verified invitation host: <code>{preview.host_id}</code>
      </p>
    </div>
  ) : null;
  const previewButton = (
    <Button
      variant={preview ? 'plain' : 'primary'}
      disabled={busy || !invite}
      onClick={() =>
        void run(
          remote
            ? {
                action: 'preview-remote',
                remote_profile: remote,
                invite,
              }
            : { action: 'preview', invite },
        )
      }
    >
      Preview
    </Button>
  );
  const requestButton = (
    <Button
      variant={preview ? 'primary' : 'plain'}
      disabled={busy || !preview}
      onClick={() => void requestMembership()}
    >
      Request membership
    </Button>
  );
  const verifyRemoteButton =
    remote && preview?.team_id ? (
      <Button
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
      </Button>
    ) : null;
  const refreshButton = (
    <Button
      disabled={busy}
      onClick={() =>
        void bridge
          .discoverGroups(profile, account)
          .then(onComplete)
          .catch((e) => setError(normalizeCommandError(e).message))
      }
    >
      Refresh memberships
    </Button>
  );
  const errorLine = error ? (
    <p role="alert" className="crit">
      {error}
    </p>
  ) : null;
  const reportsNode = (
    <>
      {reports.map((r, i) => (
        <div key={r.operation_id ?? i} role="status" className="op">
          {r.possibly_truncated && (
            <p>
              Additional requests may exist that could not be displayed. Refine
              your search or filter to view more results.
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
          {r.membership_verified && <p>Membership and team keys verified.</p>}
          {r.invite && (
            <CopyBox text={r.invite} onCopy={copy} label="Copy invitation">
              <code aria-label="Shareable invitation">{r.invite}</code>
            </CopyBox>
          )}
          {r.operation_id && !r.request_id && (
            <>
              <p>
                Operation: <code>{r.operation_id}</code>
              </p>
              {r.state === 'submission-unknown' && (
                <p>
                  The server response was not received. Checking the status will
                  not submit a duplicate request.
                </p>
              )}
              <div className="btns">
                {r.state === 'prepared' && (
                  <>
                    <Button
                      variant="primary"
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
                      Submit
                    </Button>
                    <Button
                      disabled={busy}
                      onClick={() =>
                        void run({
                          action: 'cancel',
                          operation_id: r.operation_id!,
                        })
                      }
                    >
                      Cancel
                    </Button>
                  </>
                )}
                <Button
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
                  Check status
                </Button>
              </div>
            </>
          )}
          {r.request_id && teamAlias && r.role && r.state !== 'complete' && (
            <div className="btns">
              <Button
                variant="primary"
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
                Resume approval
              </Button>
            </div>
          )}
        </div>
      ))}
    </>
  );
  // The sheet presentation covers the join flow only; the group-side
  // invitation card is reached from group settings and keeps its card.
  if (presentation && !teamAlias)
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        // Joining is a membership workflow, not a setting: the mark says
        // people, not the account panels' gear.
        glyph={
          <span className="kico invite">
            <Icon name="people" />
          </span>
        }
        footer={
          <>
            <Button disabled={busy} onClick={presentation.onClose}>
              Cancel
            </Button>
            {recoverButton}
            {refreshButton}
            {verifyRemoteButton}
            {previewButton}
            {requestButton}
          </>
        }
      >
        <p>
          Paste an invitation to request membership. You will have access once
          an administrator approves your request.
        </p>
        {errorLine}
        <Inset className="form">
          {inviteRow}
          {pinRow}
        </Inset>
        <details className="dd">
          <summary>Advanced</summary>
          <Inset className="form">
            {remoteRow}
            {sourceTeamRow}
            {sourceRoleRow}
          </Inset>
        </details>
        {previewBox}
        {reportsNode}
      </PanelSheet>
    );
  const content = (
    <section
      className="pcard"
      aria-label={
        teamAlias
          ? 'Team invitations and requests'
          : `Join a team as ${account}`
      }
    >
      <h3>
        {teamAlias ? 'Invitations and requests' : `Join a team · ${account}`}
      </h3>
      <p>
        {teamAlias
          ? 'Issue invitations and review requests to join this team.'
          : 'Paste an invitation to request membership. You will have access once an administrator approves your request.'}
      </p>
      {errorLine}
      {teamAlias ? (
        <>
          <Inset className="form">
            <InsetRow label="Role to grant">
              <select
                value={role}
                disabled={busy}
                onChange={(e) => setRole(e.target.value)}
              >
                <option value="member">Member</option>
                <option value="admin">Admin · local only</option>
                <option value="owner">Owner · local only</option>
              </select>
            </InsetRow>
            {remoteRow}
            {pinRow}
          </Inset>
          <div className="btns">
            <Button
              variant="primary"
              disabled={busy}
              onClick={() =>
                void run({ action: 'create', team_alias: teamAlias })
              }
            >
              Create invitation
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                void run({ action: 'inbox', team_alias: teamAlias })
              }
            >
              Refresh requests
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                void run({ action: 'pending-approvals', team_alias: teamAlias })
              }
            >
              Show pending approvals
            </Button>
            {recoverButton}
          </div>
          <details className="dd">
            <summary>Team nesting order</summary>
            <p>
              A team joining another team must be positioned below the
              destination team in the hierarchy. These controls change this
              team's position and validate against existing memberships.
            </p>
            <div className="btns">
              <Button
                disabled={busy}
                onClick={() =>
                  void run({
                    action: 'range',
                    team_alias: teamAlias,
                    raise: false,
                  })
                }
              >
                Lower this team's range
              </Button>
              <Button
                disabled={busy}
                onClick={() =>
                  void run({
                    action: 'range',
                    team_alias: teamAlias,
                    raise: true,
                  })
                }
              >
                Raise this team's range
              </Button>
            </div>
          </details>
          {rows.length > 0 && <SectionLabel>Membership requests</SectionLabel>}
          {rows.map((row) => (
            <article key={row.request_id} className="op">
              <p>
                {row.username ?? 'Unverified requester'} ·{' '}
                {row.joiner_kind ?? (row.remote ? 'remote' : 'local')}
              </p>
              {row.joiner_id && <code>{row.joiner_id}</code>}
              {row.error && !row.verified && <p>{row.error}</p>}
              <div className="btns">
                <Button
                  variant="primary"
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
                  Approve
                </Button>
                {row.remote && (
                  <Button
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
                    Verify on server
                  </Button>
                )}
                <Button
                  danger
                  disabled={busy}
                  onClick={() =>
                    void run({
                      action: 'reject',
                      team_alias: teamAlias,
                      request_id: row.request_id!,
                    })
                  }
                >
                  Reject
                </Button>
              </div>
            </article>
          ))}
        </>
      ) : (
        <>
          <Inset className="form">
            {inviteRow}
            {remoteRow}
            {sourceTeamRow}
            {sourceRoleRow}
            {pinRow}
          </Inset>
          {previewBox}
          <div className="btns">
            {previewButton}
            {requestButton}
            {verifyRemoteButton}
            {refreshButton}
            {recoverButton}
          </div>
        </>
      )}
      {reportsNode}
    </section>
  );
  return presentation ? (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      footer={
        <Button disabled={busy} onClick={presentation.onClose}>
          Close
        </Button>
      }
    >
      {content}
    </PanelSheet>
  ) : (
    content
  );
}
