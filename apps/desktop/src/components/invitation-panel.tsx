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
import { Button, CopyBox, Icon, Inset, InsetRow, PanelSheet } from './index';
import type { PanelPresentation } from './index';
import { serverDisplayLabel } from '../model';
import type { Server } from '../model';
import { INVITATION_ACTIVITY } from '../invitation-activity';
import { settleInvitationWrite } from '../invitation-writes';
export { INVITATION_ACTIVITY };

export type JoinServer = Pick<
  Server,
  'profileName' | 'displayLabel' | 'configuredEndpoint' | 'host_id'
>;

export function InvitationPanel({
  bridge,
  profile,
  account,
  servers,
  presentation,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  servers?: readonly JoinServer[];
  presentation?: PanelPresentation;
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
  const [joinAs, setJoinAs] = useTabSheetState('invitation.joinAs', 'self');
  const [pin, setPin] = useState('');
  const [preview, setPreview] = useState<InvitationRow | null>(null);
  const [reply, setReply] = useState<InvitationReply | null>(null);
  const [busy, setBusy] = useState(false);
  const [resumable, setResumable] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const identity = `${profile}/${account}`;
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
    setBusy(false);
    setError(null);
    return () => {
      active.current = '';
    };
  }, [identity, setInvite, setRemote, setSourceTeam, setJoinAs]);
  const nativeRole = (value: string): InvitationRole =>
    value === 'member'
      ? { member: { visibility: 0 } }
      : (value as 'admin' | 'owner');
  const run = async (action: InvitationAction) => {
    setBusy(true);
    setResumable(['attempt', 'status'].includes(action.action));
    setError(null);
    const secret = pin || null;
    setPin('');
    try {
      const value = await bridge.invitation(profile, account, action, secret);
      if (!Array.isArray(value) && value.membership_verified)
        await onComplete();
      if (active.current !== identity) return;
      setReply(value);
      if (
        !Array.isArray(value) &&
        (action.action === 'preview' || action.action === 'preview-remote')
      )
        setPreview(value);
    } catch (e) {
      if (active.current === identity)
        setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === identity) setBusy(false);
      if (!['preview', 'preview-remote', 'list'].includes(action.action))
        settleInvitationWrite(queries, profile, account);
    }
  };
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
  const twoStep = Boolean(presentation);
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
  const chooseRemote = (value: string) => {
    setRemote(value);
    setPreview(null);
  };
  const remoteRow = (
    <InsetRow label={twoStep ? 'Where is the team located?' : 'Server profile'}>
      <input
        value={remote}
        placeholder="For teams on another server"
        maxLength={128}
        disabled={busy}
        onChange={(e) => chooseRemote(e.target.value)}
      />
    </InsetRow>
  );
  const remoteChoices = (servers ?? []).filter(
    (server) => server.profileName !== profile,
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
          <option key={server.profileName} value={server.profileName}>
            {serverDisplayLabel(server)} ({server.configuredEndpoint})
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
      Show pending requests
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
            r.operation_id || r.state || r.invite || r.possibly_truncated,
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
          </div>
        ))}
    </>
  );
  if (twoStep && presentation) {
    const confirming = preview !== null;
    const host = preview?.host_id
      ? servers?.find((s) => s.host_id === preview.host_id)
      : undefined;
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
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
                  {host?.configuredEndpoint ?? preview.host_id ?? 'unreported'}
                </code>
              </p>
              {host ? (
                <p>
                  Server profile <code>{serverDisplayLabel(host)}</code>
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
  return (
    <section className="pcard" aria-label={`Join a team as ${account}`}>
      <h3>Join a team · {account}</h3>
      <p>
        Paste an invitation to request membership. You will have access once an
        administrator approves your request.
      </p>
      {errorLine}
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
      {reportsNode}
    </section>
  );
}
