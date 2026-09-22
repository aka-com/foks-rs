/** Three-step flow for previewing and submitting a team invitation. */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useDeviceCache } from '../device-cache';
import { useMetadataRepository } from '../query-hooks';
import { useSheetGuard, useTabSheetState } from '../navigation-guard';
import { normalizeCommandError, type Bridge } from '../bridge';
import type { InvitationAction, InvitationRow } from '../invitation-contract';
import type { InvitationGrantRole } from '../invitation-labels';
import {
  hardwareUnlockRequested,
  invitationRow,
  invitationRows,
  nativeRole,
  settleInvitationWrite,
} from '../invitation-writes';
import {
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  PanelSheet,
  RadioCard,
  RadioGroup,
  SectionLabel,
} from './index';
import type { PanelPresentation } from './index';
import { serverDisplayLabel } from '../model';
import type { Server, TeamStore } from '../model';
import { INVITATION_ACTIVITY } from '../invitation-activity';
export { INVITATION_ACTIVITY };

export type JoinServer = Pick<
  Server,
  'profileName' | 'displayLabel' | 'configuredEndpoint' | 'host_id'
>;

/** Team that the current account may use as the joining member. */
export type JoinTeam = Pick<TeamStore, 'id' | 'alias' | 'name'>;

type Step = 'invite' | 'confirm' | 'sent' | 'pending';

/** Indicates that the operation requires a security key PIN. */
class UnlockNeeded extends Error {
  constructor() {
    super('Enter the PIN for your security key to continue.');
  }
}

/** Returns user-facing text for a join-request state. */
function requestStateText(row: InvitationRow): string {
  if (row.membership_verified) return 'Membership and team keys verified.';
  switch (row.state) {
    case 'prepared':
      return 'Not sent yet.';
    case 'submitting':
      return 'Sending.';
    case 'submission-unknown':
      return 'Sent, but the server response was not received. Checking the status will not send a duplicate.';
    case 'cancelled':
      return 'Cancelled.';
    default:
      return row.delivery_acknowledged || row.state === 'complete'
        ? 'Delivered. An administrator of the team must approve it.'
        : (row.state ?? 'Unknown');
  }
}

function requestChip(row: InvitationRow): ReactNode {
  if (row.membership_verified) return <Chip tone="ok">Member</Chip>;
  switch (row.state) {
    case 'prepared':
      return <Chip tone="warn">Not sent</Chip>;
    case 'submission-unknown':
      return <Chip tone="bad">Unknown</Chip>;
    case 'cancelled':
      return <Chip>Cancelled</Chip>;
    default:
      return <Chip tone="warn">Pending</Chip>;
  }
}

export function InvitationPanel({
  bridge,
  profile,
  account,
  servers,
  teams,
  presentation,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  /** Configured profiles available in the server picker. */
  servers?: readonly JoinServer[];
  /** Administered teams that may join instead of the current account. */
  teams?: readonly JoinTeam[];
  presentation: PanelPresentation;
  onComplete: () => Promise<void> | void;
}): ReactNode {
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
  const [invite, setInvite] = useTabSheetState('invitation.invite', '');
  const [remote, setRemote] = useTabSheetState('invitation.remote', '');
  const [sourceTeam, setSourceTeam] = useTabSheetState(
    'invitation.sourceTeam',
    '',
  );
  const [sourceRole, setSourceRole] = useTabSheetState<InvitationGrantRole>(
    'invitation.sourceRole',
    'admin',
  );
  const [joinAs, setJoinAs] = useTabSheetState<'self' | 'team'>(
    'invitation.joinAs',
    'self',
  );
  const [pin, setPin] = useState('');
  const [needPin, setNeedPin] = useState(false);
  const [preview, setPreview] = useState<InvitationRow | null>(null);
  const [operation, setOperation] = useState<InvitationRow | null>(null);
  const [pending, setPending] = useState<InvitationRow[] | null>(null);
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
    setNeedPin(false);
    setPreview(null);
    setOperation(null);
    setPending(null);
    setBusy(false);
    setError(null);
    return () => {
      active.current = '';
    };
  }, [identity, setInvite, setRemote, setSourceTeam, setJoinAs]);

  const step: Step = pending
    ? 'pending'
    : operation
      ? 'sent'
      : preview
        ? 'confirm'
        : 'invite';

  const call = (action: InvitationAction) =>
    bridge.invitation(profile, account, action, pin || null);

  /** Runs an invitation operation and updates busy, error, and recovery state. */
  const run = async (
    work: () => Promise<void>,
    options: { write: boolean; resumable?: boolean },
  ): Promise<void> => {
    setBusy(true);
    setResumable(Boolean(options.resumable));
    setError(null);
    try {
      await work();
      if (active.current !== identity) return;
      setPin('');
      setNeedPin(false);
    } catch (failure) {
      if (active.current !== identity) return;
      if (failure instanceof UnlockNeeded || hardwareUnlockRequested(failure)) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to continue.');
      } else setError(normalizeCommandError(failure).message);
    } finally {
      if (active.current === identity) setBusy(false);
      if (options.write) settleInvitationWrite(queries, profile, account);
    }
  };

  /** Throws when the response requires a security key PIN. */
  const unlocked = (row: InvitationRow): boolean => {
    if (row.hardware_required) throw new UnlockNeeded();
    return true;
  };

  const runPreview = () =>
    run(
      async () => {
        const row = invitationRow(
          await call(
            remote
              ? { action: 'preview-remote', remote_profile: remote, invite }
              : { action: 'preview', invite },
          ),
        );
        if (active.current === identity) setPreview(row);
      },
      { write: false },
    );

  const sourceAlias = joinAs === 'team' ? sourceTeam : '';
  const attemptOf = (row: InvitationRow): InvitationAction =>
    row.remote
      ? {
          action: 'attempt-remote',
          remote_profile: row.remote_profile ?? remote,
          operation_id: row.operation_id!,
        }
      : { action: 'attempt', operation_id: row.operation_id! };
  const statusOf = (row: InvitationRow): InvitationAction =>
    row.remote
      ? {
          action: 'status-remote',
          remote_profile: row.remote_profile ?? remote,
          operation_id: row.operation_id!,
        }
      : { action: 'status', operation_id: row.operation_id! };

  /** Prepares and submits a membership request. */
  const requestMembership = () =>
    run(
      async () => {
        const accept: InvitationAction = sourceAlias
          ? remote
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
              }
          : remote
            ? { action: 'accept-remote', remote_profile: remote, invite }
            : { action: 'accept', invite };
        let row: InvitationRow = {
          ...invitationRow(await call(accept)),
          remote: Boolean(remote),
          remote_profile: remote || undefined,
        };
        if (active.current !== identity) return;
        unlocked(row);
        setOperation(row);
        if (row.state === 'prepared' && row.operation_id) {
          row = { ...row, ...invitationRow(await call(attemptOf(row))) };
          if (active.current !== identity) return;
          unlocked(row);
          setOperation(row);
        }
        if (row.membership_verified) await onComplete();
      },
      { write: true, resumable: true },
    );

  /** Updates matching request state in the active and pending views. */
  const absorb = (row: InvitationRow): void => {
    setOperation((current) =>
      current?.operation_id === row.operation_id ? row : current,
    );
    setPending((rows) =>
      rows
        ? rows.map((r) => (r.operation_id === row.operation_id ? row : r))
        : rows,
    );
  };
  const submit = (row: InvitationRow) =>
    run(
      async () => {
        // Progress replies may omit the saved request's destination.
        const current = {
          ...row,
          ...invitationRow(await call(attemptOf(row))),
        };
        if (active.current !== identity) return;
        unlocked(current);
        absorb(current);
        if (current.membership_verified) await onComplete();
      },
      { write: true, resumable: true },
    );
  const checkStatus = (row: InvitationRow) =>
    run(
      async () => {
        const current = { ...row, ...invitationRow(await call(statusOf(row))) };
        if (active.current !== identity) return;
        unlocked(current);
        absorb(current);
        if (current.membership_verified) await onComplete();
      },
      { write: false, resumable: true },
    );
  const cancelRequest = (row: InvitationRow) =>
    run(
      async () => {
        await call({ action: 'cancel', operation_id: row.operation_id! });
        if (active.current !== identity) return;
        setOperation((current) =>
          current?.operation_id === row.operation_id ? null : current,
        );
        setPending((rows) =>
          rows ? rows.filter((r) => r.operation_id !== row.operation_id) : rows,
        );
      },
      { write: true },
    );
  const verifyRemote = () =>
    run(
      async () => {
        if (!preview?.team_id) return;
        const current = invitationRow(
          await call({
            action: 'sync-remote',
            remote_profile: remote,
            team_id: preview.team_id,
            ...(sourceAlias
              ? {
                  source_team_alias: sourceAlias,
                  source_role: nativeRole(sourceRole),
                }
              : {}),
          }),
        );
        if (active.current !== identity) return;
        unlocked(current);
        setOperation((row) => (row ? { ...row, ...current } : current));
        if (current.membership_verified) await onComplete();
      },
      { write: true },
    );
  const loadPending = () =>
    run(
      async () => {
        const rows = invitationRows(await call({ action: 'list' }));
        if (active.current !== identity) return;
        // Delivery completes before membership approval. Keep delivered requests
        // reachable so their status can still be checked after reopening.
        setPending(
          rows.filter(
            (row) =>
              row.operation_id &&
              !row.request_id &&
              row.state !== 'cancelled' &&
              !row.membership_verified,
          ),
        );
      },
      { write: false },
    );

  // Prepared requests persist in the agent and are resumable. Confirm before
  // discarding invitation text that has not been prepared.
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
              presentation.onClose();
            },
          }
        : null,
    !busy && !pin,
  );

  const errorLine = error ? (
    <p role="alert" className="crit">
      {error}
    </p>
  ) : null;
  const pinRow = needPin ? (
    <Inset className="form">
      <InsetRow label="Security key PIN">
        <input
          type="password"
          autoComplete="off"
          maxLength={32}
          value={pin}
          disabled={busy}
          onChange={(event) => setPin(event.target.value)}
        />
      </InsetRow>
    </Inset>
  ) : null;
  const glyph = (
    <span className="kico invite">
      <Icon name="users" />
    </span>
  );
  const remoteChoices = (servers ?? []).filter(
    (server) => server.profileName !== profile,
  );
  const chooseRemote = (value: string): void => {
    setRemote(value);
    setPreview(null);
  };
  const host = preview?.host_id
    ? servers?.find((server) => server.host_id === preview.host_id)
    : undefined;
  const teamLabel = (row: InvitationRow): string =>
    row.name ?? (row.team_id ? `Team ${row.team_id.slice(0, 12)}…` : 'Team');

  /** Renders a request and its currently available actions. */
  const requestRow = (row: InvitationRow, primary: boolean): ReactNode => (
    <article
      key={row.operation_id}
      className="op"
      aria-label={`Request to join ${teamLabel(row)}`}
    >
      <p>
        <b>{teamLabel(row)}</b> {requestChip(row)}
      </p>
      <p className="fn">{requestStateText(row)}</p>
      <div className="btns">
        {row.state === 'prepared' ? (
          <>
            <Button
              size="sm"
              variant={primary ? 'primary' : 'plain'}
              disabled={busy}
              onClick={() => void submit(row)}
            >
              Submit
            </Button>
            <Button
              size="sm"
              disabled={busy}
              onClick={() => void cancelRequest(row)}
            >
              Cancel
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant={primary ? 'primary' : 'plain'}
            disabled={busy}
            onClick={() => void checkStatus(row)}
          >
            Check status
          </Button>
        )}
      </div>
    </article>
  );

  if (step === 'pending' && pending)
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        glyph={glyph}
        title="Pending requests"
        footer={
          <>
            <Button
              icon="refresh"
              disabled={busy}
              onClick={() => void loadPending()}
            >
              Refresh
            </Button>
            <span className="spacer" />
            <Button disabled={busy} onClick={() => setPending(null)}>
              Back
            </Button>
          </>
        }
      >
        <p>Requests this account started that have not been approved yet.</p>
        {errorLine}
        {pinRow}
        {pending.length ? (
          pending.map((row) => requestRow(row, true))
        ) : (
          <div className="callout">
            <span className="kico neutral">
              <Icon name="check" />
            </span>
            <span className="t">
              <b>No pending requests.</b>
            </span>
          </div>
        )}
      </PanelSheet>
    );

  if (step === 'sent' && operation) {
    const prepared = operation.state === 'prepared';
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        glyph={glyph}
        step="Step 3 of 3"
        title={
          operation.membership_verified
            ? 'You are a member'
            : prepared
              ? 'Request not sent'
              : 'Request sent'
        }
        footer={
          prepared ? (
            <>
              <Button
                disabled={busy}
                onClick={() => void cancelRequest(operation)}
              >
                Cancel request
              </Button>
              <span className="spacer" />
              <Button
                variant="primary"
                disabled={busy}
                onClick={() => void submit(operation)}
              >
                Submit
              </Button>
            </>
          ) : (
            <>
              {operation.membership_verified ? null : (
                <Button
                  disabled={busy}
                  onClick={() => void checkStatus(operation)}
                >
                  Check status
                </Button>
              )}
              {remote && preview?.team_id && !operation.membership_verified ? (
                <Button disabled={busy} onClick={() => void verifyRemote()}>
                  Verify membership and keys
                </Button>
              ) : null}
              <span className="spacer" />
              <Button
                variant="primary"
                disabled={busy}
                onClick={presentation.onClose}
              >
                Done
              </Button>
            </>
          )
        }
      >
        <Inset className="form">
          <InsetRow label="Team" action={requestChip(operation)}>
            <b>{preview?.name ?? teamLabel(operation)}</b>
          </InsetRow>
          <InsetRow label="Requesting as">
            {sourceAlias
              ? `${teams?.find((team) => team.alias === sourceAlias)?.name ?? sourceAlias} (${sourceRole})`
              : account}
          </InsetRow>
          <InsetRow label="Status">
            <span role="status">{requestStateText(operation)}</span>
          </InsetRow>
        </Inset>
        {errorLine}
        {pinRow}
        <p className="fn">
          {operation.membership_verified
            ? `${preview?.name ?? 'The team'} is now in your Teams list.`
            : prepared
              ? 'The request is saved on this device. Submit sends it to the server.'
              : `You can close this and come back through Resume a pending request.${
                  remote
                    ? ' Once approved, Verify membership and keys confirms the membership against the team’s server.'
                    : ''
                }`}
        </p>
      </PanelSheet>
    );
  }

  if (step === 'confirm' && preview) {
    const teamChoices = teams ?? [];
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        glyph={glyph}
        step="Step 2 of 3"
        title="Confirm this team"
        footer={
          <>
            <Button disabled={busy} onClick={() => setPreview(null)}>
              Back
            </Button>
            <span className="spacer" />
            <Button disabled={busy} onClick={presentation.onClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={busy || (joinAs === 'team' && !sourceTeam)}
              onClick={() => void requestMembership()}
            >
              Request membership
            </Button>
          </>
        }
      >
        <Inset className="form">
          <InsetRow label="Team" action={<Chip tone="ok">Verified</Chip>}>
            <b>{preview.name ?? 'Unnamed team'}</b>
          </InsetRow>
          <InsetRow label="Team ID">
            <code>{preview.team_id}</code>
          </InsetRow>
          <InsetRow label="Host">
            <code>
              {host?.configuredEndpoint ?? preview.host_id ?? 'unreported'}
            </code>
            {host ? (
              <small>Server profile {serverDisplayLabel(host)}</small>
            ) : null}
          </InsetRow>
        </Inset>
        {errorLine}
        <SectionLabel>Request access as</SectionLabel>
        <Inset>
          <RadioGroup label="Request access as">
            <RadioCard
              selected={joinAs === 'self'}
              onSelect={() => setJoinAs('self')}
              title="Myself"
              detail={`${account} joins ${preview.name ?? 'the team'} at the role an administrator grants.`}
            />
            <RadioCard
              selected={joinAs === 'team'}
              onSelect={() => setJoinAs('team')}
              title="A team I administer"
              detail={`That whole team becomes a member of ${preview.name ?? 'the team'}. Everyone in it gets access.`}
            />
          </RadioGroup>
        </Inset>
        {joinAs === 'team' ? (
          <Inset className="form">
            <InsetRow label="Team to add">
              {teamChoices.length ? (
                <select
                  value={sourceTeam}
                  disabled={busy}
                  onChange={(event) => setSourceTeam(event.target.value)}
                >
                  <option value="">Choose a team</option>
                  {teamChoices.map((team) => (
                    <option key={team.id} value={team.alias}>
                      {team.name}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={sourceTeam}
                  placeholder="The team to add, by its local name"
                  maxLength={128}
                  disabled={busy}
                  onChange={(event) => setSourceTeam(event.target.value)}
                />
              )}
            </InsetRow>
            <InsetRow label="Your role in it">
              <select
                value={sourceRole}
                disabled={busy}
                onChange={(event) =>
                  setSourceRole(event.target.value as InvitationGrantRole)
                }
              >
                <option value="member">Member</option>
                <option value="admin">Admin</option>
                <option value="owner">Owner</option>
              </select>
            </InsetRow>
          </Inset>
        ) : null}
        {pinRow}
        <p className="fn">
          An administrator of {preview.name ?? 'the team'} must approve the
          request before{' '}
          {joinAs === 'team'
            ? (teamChoices.find((team) => team.alias === sourceTeam)?.name ??
              'the team')
            : 'you'}{' '}
          {joinAs === 'team' ? 'has' : 'have'} access.
        </p>
      </PanelSheet>
    );
  }

  return (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      glyph={glyph}
      step="Step 1 of 3"
      footer={
        <>
          <button
            type="button"
            className="lnk"
            disabled={busy}
            onClick={() => void loadPending()}
          >
            Resume a pending request
          </button>
          <span className="spacer" />
          <Button disabled={busy} onClick={presentation.onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={busy || !invite.trim()}
            onClick={() => void runPreview()}
          >
            Continue
          </Button>
        </>
      }
    >
      <p>Paste the invitation an administrator of the team sent you.</p>
      {errorLine}
      <Inset className="form">
        <InsetRow label="Invitation">
          <input
            value={invite}
            placeholder="Paste an invitation"
            style={{ minWidth: 0, textOverflow: 'ellipsis' }}
            maxLength={256}
            disabled={busy}
            onChange={(event) => {
              setInvite(event.target.value);
              setPreview(null);
            }}
          />
        </InsetRow>
        {servers ? (
          <InsetRow label="Where is the team located?">
            <select
              value={remote}
              disabled={busy}
              onChange={(event) => chooseRemote(event.target.value)}
            >
              <option value="">This account’s server</option>
              {remoteChoices.map((server) => (
                <option key={server.profileName} value={server.profileName}>
                  {serverDisplayLabel(server)} ({server.configuredEndpoint})
                </option>
              ))}
            </select>
          </InsetRow>
        ) : (
          <InsetRow label="Where is the team located?">
            <input
              value={remote}
              placeholder="For teams on another server"
              maxLength={128}
              disabled={busy}
              onChange={(event) => chooseRemote(event.target.value)}
            />
          </InsetRow>
        )}
      </Inset>
      {pinRow}
      <p className="fn">
        An invitation is one line of letters and digits. It is checked against
        the team’s server before anything is sent.
      </p>
    </PanelSheet>
  );
}
