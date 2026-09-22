import { useDeviceCache } from '../device-cache';
import { useMetadataRepository } from '../query-hooks';
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
import { serverDisplayName } from '../model';
import type { Server } from '../model';
import { INVITATION_ACTIVITY } from '../invitation-activity';
import { pendingOperationKey } from '../operation-queries';
export { INVITATION_ACTIVITY };

/** The part of a configured profile the join sheet's server picker needs. */
export type JoinServer = Pick<
  Server,
  'name' | 'label' | 'configuredProbe' | 'host_id'
>;

export function InvitationPanel({
  bridge,
  profile,
  account,
  teamAlias,
  teamId,
  servers,
  presentation,
  recover = false,
  requestsOnly = false,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  teamAlias?: string;
  teamId?: string;
  /**
   * The locally configured profiles, for the join sheet's server picker. A
   * profile is a trust-pinned host under a local name, so the set is finite
   * and known; without it the sheet falls back to naming a profile by hand.
   */
  servers?: readonly JoinServer[];
  /**
   * Render the join flow as a modal sheet instead of a card: the form in the
   * body and its actions in the footer.
   */
  presentation?: PanelPresentation;
  recover?: boolean;
  /**
   * Draw the membership requests alone: the label and one row per request,
   * without the form that issues invitations. The team page's Requests tab
   * is a list of decisions to take; issuing an invitation is an add action
   * and is reached from its Add people control, which opens this same panel
   * as a sheet.
   */
  requestsOnly?: boolean;
  onComplete: () => Promise<void> | void;
}) {
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
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
  const [joinAs, setJoinAs] = useTabSheetState('invitation.joinAs', 'self');
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
      setJoinAs('self');
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
  }, [identity, setInvite, setRemote, setSourceTeam, setJoinAs]);
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
  // The team side has no modal presentation and is mounted for the life of
  // the team page, not just while its tab is open, so its own request list
  // is loaded once up front instead of waiting for a manual Refresh requests.
  useEffect(() => {
    if (presentation || !teamAlias) return;
    let live = true;
    void bridge
      .invitation(
        profile,
        account,
        { action: 'inbox', team_alias: teamAlias },
        null,
      )
      .then((value) => {
        if (live && !Array.isArray(value) && value.rows) setRows(value.rows);
      })
      .catch(() => {
        // A silent background load; Refresh requests surfaces its own error.
      });
    return () => {
      live = false;
    };
  }, [bridge, profile, account, teamAlias, presentation]);
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
        // Invalidate even when the recovery banner is currently unmounted,
        // and the shared request count with it, so the Requests tab, the
        // Teams list and the rail follow the decision this panel just made.
        queries.invalidate(['invitation-recovery', profile, account]);
        queries.invalidate(['team-requests', profile, account]);
        // Adding a federated team is a membership write, so it can journal a resumable
        // operation on the team it admits into. A team page open elsewhere
        // holds that journal, and this is the only writer of it that is not
        // already one of that page's own actions.
        queries.invalidate(pendingOperationKey(profile));
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
  // The join flow is the two-step sheet; the group-side card and the
  // non-modal join card keep a single form, so the "Requesting as" select
  // that gates the source team exists only in the sheet.
  const twoStep = Boolean(presentation) && !teamAlias;
  const sourceAlias = !twoStep || joinAs === 'team' ? sourceTeam : '';
  const requestMembership = () => {
    if (sourceAlias)
      return run(
        remote
          ? {
              action: 'accept-team-remote',
              remote_profile: remote,
              invite,
              source_team_alias: sourceAlias,
              source_role: nativeRole(sourceRole),
            }
          : {
              action: 'accept-team',
              invite,
              source_team_alias: sourceAlias,
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
    <InsetRow label={twoStep ? 'Where is the team located?' : 'Server profile'}>
      <input
        value={remote}
        placeholder="For teams on another server"
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
  const chooseRemote = (value: string) => {
    setRemote(value);
    setPreview(null);
    setRows((r) => r.map((x) => (x.remote ? { ...x, verified: false } : x)));
  };
  // A profile is a locally named, trust-pinned host, so the set is finite and
  // already known: the sheet picks from it rather than asking for the name
  // from memory, where a typo only fails later, against the host.
  const remoteChoices = (servers ?? []).filter(
    (server) => server.name !== profile,
  );
  const serverRow = servers ? (
    <InsetRow label="Where is the team located?">
      <select
        value={remote}
        disabled={busy}
        onChange={(e) => chooseRemote(e.target.value)}
      >
        <option value="">This account's server</option>
        {remoteChoices.map((server) => (
          <option key={server.name} value={server.name}>
            {serverDisplayName(server)} ({server.configuredProbe})
          </option>
        ))}
      </select>
    </InsetRow>
  ) : (
    remoteRow
  );
  const pinRow = (
    <InsetRow label="Security key PIN">
      <input
        type="password"
        autoComplete="off"
        placeholder={
          twoStep ? 'If using a hardware security key' : 'Enrolled keys only'
        }
        maxLength={32}
        value={pin}
        disabled={busy}
        onChange={(e) => setPin(e.target.value)}
      />
    </InsetRow>
  );
  const recoverButton = twoStep ? (
    <button
      type="button"
      className="lnk"
      disabled={busy}
      onClick={() => void run({ action: 'list' })}
    >
      Resume a request
    </button>
  ) : (
    <Button disabled={busy} onClick={() => void run({ action: 'list' })}>
      {teamAlias ? 'Show pending operations' : 'Show pending requests'}
    </Button>
  );
  const inviteRow = (
    <InsetRow label="Invitation">
      <input
        value={invite}
        placeholder="Paste an invitation"
        style={{ minWidth: 0, textOverflow: 'ellipsis' }}
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
    <InsetRow
      label={
        twoStep
          ? 'Team to add'
          : 'Requesting team (to add a team you administer instead of yourself)'
      }
    >
      <input
        value={sourceTeam}
        placeholder={twoStep ? 'The team to add, by its local name' : undefined}
        maxLength={128}
        disabled={busy}
        onChange={(e) => setSourceTeam(e.target.value)}
      />
    </InsetRow>
  );
  const sourceRoleRow = sourceTeam ? (
    <InsetRow label={twoStep ? 'Your role in it' : 'Requesting team role'}>
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
  const runPreview = () =>
    void run(
      remote
        ? { action: 'preview-remote', remote_profile: remote, invite }
        : { action: 'preview', invite },
    );
  const previewButton = (
    <Button
      variant={preview ? 'plain' : 'primary'}
      disabled={busy || !invite}
      onClick={runPreview}
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
            ...(sourceAlias
              ? {
                  source_team_alias: sourceAlias,
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
      {reports
        .filter((r) =>
          Boolean(
            r.operation_id ||
            r.request_id ||
            r.state ||
            r.invite ||
            r.possibly_truncated,
          ),
        )
        .map((r, i) => (
          <div key={r.operation_id ?? i} role="status" className="op">
            {r.possibly_truncated && (
              <p>
                Additional requests may exist that could not be displayed.
                Refine your search or filter to view more results.
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
                    The server response was not received. Checking the status
                    will not submit a duplicate request.
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
  // One request, wherever the list is drawn. A remote request names the
  // server it came from, so it is approved and verified against that profile
  // rather than against whatever the invitation form last had typed in it.
  const alias = teamAlias;
  // The role select lives on the invitation form, which the Requests tab
  // does not draw, and its value is kept per page, so the tab cannot approve
  // at a role the reader never saw: there it admits as Member and its button
  // says so. A higher role is granted afterwards from the Members list.
  const grantRole = requestsOnly ? 'member' : role;
  const requestRows = !alias
    ? []
    : rows.map((row) => {
        const rowRemote = row.source_profile ?? row.remote_profile ?? remote;
        return (
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
                  (!!row.remote && (!rowRemote || grantRole !== 'member'))
                }
                onClick={() =>
                  void run(
                    row.remote
                      ? {
                          action: 'approve-remote',
                          remote_profile: rowRemote,
                          team_alias: alias,
                          request_id: row.request_id!,
                          role: nativeRole(grantRole),
                        }
                      : {
                          action: 'approve',
                          team_alias: alias,
                          request_id: row.request_id!,
                          role: nativeRole(grantRole),
                        },
                  )
                }
              >
                {requestsOnly ? 'Approve as Member' : 'Approve'}
              </Button>
              {row.remote && (
                <Button
                  disabled={busy || !rowRemote}
                  onClick={() =>
                    void run({
                      action: 'inspect-remote',
                      remote_profile: rowRemote,
                      team_alias: alias,
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
                    team_alias: alias,
                    request_id: row.request_id!,
                  })
                }
              >
                Reject
              </Button>
            </div>
          </article>
        );
      });
  const requestList = (
    <>
      {rows.length > 0 && <SectionLabel>Membership requests</SectionLabel>}
      {requestRows}
    </>
  );
  // The Requests tab asks for the decisions only. The label stays even with
  // nothing pending, so the tab names what it is rather than reading as a
  // page that failed to load.
  if (requestsOnly)
    return (
      <section className="pcard" aria-label="Membership requests">
        {errorLine}
        <SectionLabel>Membership requests</SectionLabel>
        {rows.length ? (
          requestRows
        ) : (
          <p>No one is waiting to join this team.</p>
        )}
        {reportsNode}
      </section>
    );
  // The sheet presentation covers the join flow only; the group-side
  // invitation card is reached from group settings and keeps its card.
  // The join sheet is two steps: paste the invitation, then confirm which team
  // and host it resolved to before anything is sent. The step is the preview
  // itself rather than a separate counter, so editing the invitation — which
  // already clears the preview — cannot leave the sheet confirming a team the
  // text no longer names.
  if (twoStep && presentation) {
    const confirming = preview !== null;
    const host = preview?.host_id
      ? servers?.find((s) => s.host_id === preview.host_id)
      : undefined;
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        // Joining is a membership workflow, not a setting: the mark says
        // people, not the account panels' gear.
        glyph={
          <span className="kico invite">
            <Icon name="users" />
          </span>
        }
        step={confirming ? 'Step 2 of 2' : 'Step 1 of 2'}
        title={confirming ? 'Confirm this team' : undefined}
        footer={
          confirming ? (
            <>
              <Button disabled={busy} onClick={() => setPreview(null)}>
                Back
              </Button>
              <Button
                variant="primary"
                disabled={busy || (joinAs === 'team' && !sourceTeam)}
                onClick={() => void requestMembership()}
              >
                Request membership
              </Button>
            </>
          ) : (
            <>
              {recoverButton}
              <span className="spacer" />
              <Button disabled={busy} onClick={presentation.onClose}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={busy || !invite}
                onClick={runPreview}
              >
                Continue
              </Button>
            </>
          )
        }
      >
        {confirming ? (
          <>
            {errorLine}
            <div className="op" role="status">
              <p>
                <strong>{preview.name ?? 'Unnamed team'}</strong>
              </p>
              <p>
                Team <code>{preview.team_id}</code>
              </p>
              <p>
                Verified host{' '}
                <code>
                  {host?.configuredProbe ?? preview.host_id ?? 'unreported'}
                </code>
              </p>
              {host ? (
                <p>
                  Server profile <code>{serverDisplayName(host)}</code>
                </p>
              ) : null}
            </div>
            <Inset className="form">
              <InsetRow label="Request access as">
                <select
                  value={joinAs}
                  disabled={busy}
                  onChange={(e) => setJoinAs(e.target.value)}
                >
                  <option value="self">Myself ({account})</option>
                  <option value="team">A team I administer</option>
                </select>
              </InsetRow>
              {joinAs === 'team' ? sourceTeamRow : null}
              {joinAs === 'team' ? sourceRoleRow : null}
              {pinRow}
            </Inset>
            <p>This join request must be approved by an administrator.</p>
            {verifyRemoteButton ? (
              <div className="btns">{verifyRemoteButton}</div>
            ) : null}
            {reportsNode}
          </>
        ) : (
          <>
            <p>Paste an invite from another team administrator.</p>
            {errorLine}
            <Inset className="form">
              {inviteRow}
              {serverRow}
              {pinRow}
            </Inset>
            {reportsNode}
          </>
        )}
      </PanelSheet>
    );
  }
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
          {requestList}
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
