import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useDeviceCache } from '../device-cache';
import type { InvitationAction, InvitationRow } from '../invitation-contract';
import {
  forgetInvitationLabel,
  invitationLabel,
  rememberInvitationLabel,
  relativeTime,
} from '../invitation-labels';
import type { InvitationGrantRole } from '../invitation-labels';
import {
  grantRoleName,
  hardwareUnlockRequested,
  invitationInstructions,
  invitationRow,
  invitationRows,
  operationStateText,
  settleInvitationWrite,
  unfinishedOperation,
} from '../invitation-writes';
import type { TeamStore } from '../model';
import { useMetadataRepository } from '../query-hooks';
import { useSheetGuard, useTabSheetState } from '../navigation-guard';
import {
  Band,
  Button,
  Chip,
  CopyBox,
  Icon,
  Inset,
  InsetRow,
  Notice,
  PanelSheet,
  RadioCard,
  RadioGroup,
  SectionLabel,
} from './index';

type ApprovalRole = Extract<InvitationGrantRole, 'member' | 'admin'>;

export function InviteNewUserSheet({
  bridge,
  team,
  serverLabel,
  approver,
  onClose,
  onComplete,
}: {
  bridge: Bridge;
  team: TeamStore;
  serverLabel: string;
  approver?: string;
  onClose: () => void;
  onComplete?: () => Promise<void> | void;
}): ReactNode {
  const profile = team.server;
  const account = team.account;
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
  const [label, setLabel] = useTabSheetState('invite.label', '');
  const [role, setRole] = useTabSheetState<ApprovalRole>(
    'invite.role',
    'member',
  );
  const [pin, setPin] = useState('');
  const [needPin, setNeedPin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [operation, setOperation] = useState<InvitationRow | null>(null);
  const [unfinished, setUnfinished] = useState<InvitationRow[]>([]);
  const [copied, setCopied] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    let mounted = true;
    void bridge
      .invitation(profile, account, { action: 'list' }, null)
      .then((reply) => {
        if (!mounted) return;
        setUnfinished(
          invitationRows(reply).filter(
            (row) =>
              row.team_id === team.team_id_hex && unfinishedOperation(row),
          ),
        );
      })
      .catch(() => undefined);
    return () => {
      mounted = false;
    };
  }, [bridge, profile, account, team.team_id_hex]);

  const call = (action: InvitationAction) =>
    bridge
      .invitation(profile, account, action, pin || null)
      .then(invitationRow);

  const shared = operation?.state === 'complete' && operation.invite;

  const finish = async (row: InvitationRow): Promise<void> => {
    const id = row.operation_id;
    if (!id) return;
    // Resumed rows need the same durable identity as newly prepared rows,
    // including when the request fails before returning an updated state.
    setOperation(row);
    let current = row;
    if (current.state === 'prepared' || current.state === 'submitting') {
      current = await call({ action: 'attempt', operation_id: id });
    } else if (
      current.state === 'submission-unknown' ||
      current.state === 'acknowledged'
    ) {
      current = await call({ action: 'status', operation_id: id });
    }
    if (!live.current) return;
    if (current.hardware_required) {
      setNeedPin(true);
      setOperation({ ...row, operation_id: id });
      setError('Enter the PIN for your security key to finish creating.');
      return;
    }
    setPin('');
    setNeedPin(false);
    setOperation(current);
    setUnfinished((rows) => rows.filter((r) => r.operation_id !== id));
    if (current.state === 'complete' && current.invite)
      await refreshAfterCompletion();
  };

  const refreshAfterCompletion = async (): Promise<void> => {
    try {
      await onComplete?.();
    } catch (failure) {
      if (live.current)
        setError(
          `The invitation is ready, but refreshing the team failed: ${normalizeCommandError(failure).message}`,
        );
    }
  };

  const run = async (work: () => Promise<void>): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      if (!live.current) return;
      if (hardwareUnlockRequested(failure)) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to sign the invitation.');
      } else setError(normalizeCommandError(failure).message);
    } finally {
      if (live.current) setBusy(false);
      settleInvitationWrite(queries, profile, account);
    }
  };

  const create = () =>
    run(async () => {
      const row = operation?.operation_id
        ? operation
        : await call({ action: 'create', team_alias: team.alias });
      // Save the operation ID before submission so a retry resumes this invitation.
      if (live.current) setOperation(row);
      if (row.operation_id && !operation?.operation_id)
        rememberInvitationLabel(profile, account, row.operation_id, {
          label: label.trim(),
          role,
          created: Date.now(),
        });
      await finish(row);
    });

  const discard = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      await call({ action: 'cancel', operation_id: row.operation_id });
      forgetInvitationLabel(profile, account, row.operation_id);
      if (!live.current) return;
      setUnfinished((rows) =>
        rows.filter((r) => r.operation_id !== row.operation_id),
      );
      if (operation?.operation_id === row.operation_id) setOperation(null);
    });

  const checkStatus = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      setOperation(row);
      const current = await call({
        action: 'status',
        operation_id: row.operation_id,
      });
      if (!live.current) return;
      if (current.hardware_required) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to check the status.');
        return;
      }
      setOperation(current);
      setUnfinished((rows) =>
        rows
          .map((r) => (r.operation_id === row.operation_id ? current : r))
          .filter(unfinishedOperation),
      );
      if (current.state === 'complete' && current.invite)
        await refreshAfterCompletion();
    });

  const copy = (text: string, what: string) =>
    void bridge
      .copyText(text)
      .then(() => {
        if (live.current) setCopied(what);
      })
      .catch((e) => setError(normalizeCommandError(e).message));

  const reset = () => {
    setOperation(null);
    setLabel('');
    setRole('member');
    setPin('');
    setNeedPin(false);
    setError(null);
    setCopied(null);
  };

  useSheetGuard(
    busy
      ? null
      : label.trim() && !operation
        ? {
            verdict: 'prompt',
            title: 'Discard this invitation?',
            body: `The note and role you chose have not been used yet. Nothing has been sent to ${serverLabel}.`,
            confirm: 'Discard',
            onConfirm: () => {
              reset();
              onClose();
            },
          }
        : null,
    !busy && !pin,
  );

  const glyph = (
    <span className="kico invite">
      <Icon name="door" />
    </span>
  );
  const errorLine = error ? (
    <p role="alert" className="crit">
      {error}
    </p>
  ) : null;
  const pinRow = needPin ? (
    <InsetRow label="Security key PIN">
      <input
        type="password"
        autoComplete="off"
        maxLength={32}
        value={pin}
        disabled={busy}
        onChange={(e) => setPin(e.target.value)}
      />
    </InsetRow>
  ) : null;

  if (shared && operation) {
    const note = operation.operation_id
      ? invitationLabel(profile, account, operation.operation_id)
      : undefined;
    const instructions = invitationInstructions({
      team: team.name,
      server: serverLabel,
      invite: operation.invite!,
      approver,
    });
    return (
      <PanelSheet
        presentation={{ title: 'Invite a new user', onClose }}
        title="Share the invitation"
        step="Step 2 of 2"
        busy={busy}
        glyph={glyph}
        footer={
          <>
            <button
              type="button"
              className="lnk"
              disabled={busy}
              onClick={reset}
            >
              Create another
            </button>
            <span className="spacer" />
            <Button variant="primary" disabled={busy} onClick={onClose}>
              Done
            </Button>
          </>
        }
      >
        <Band severity="info" live label="Invitation ready">
          {note?.label ? `for ${note.label} · ` : ''}
          {grantRoleName(note?.role ?? role)} on approval
        </Band>
        {errorLine}
        {copied ? (
          <p role="status" className="fn">
            {copied} copied.
          </p>
        ) : null}
        <CopyBox
          text={operation.invite!}
          onCopy={(text) => copy(text, 'Invitation')}
          label="Copy"
        >
          <code aria-label="Shareable invitation">{operation.invite}</code>
        </CopyBox>
        <SectionLabel>What to send them</SectionLabel>
        <CopyBox
          text={instructions}
          onCopy={(text) => copy(text, 'Instructions')}
          label="Copy"
        >
          <pre className="instructions">{instructions}</pre>
        </CopyBox>
        <p className="fn">
          If {serverLabel} requires an invite code to create accounts, they will
          also need one from the server operator. Anyone with this invitation
          can request to join, so send it privately. Requests still need your
          approval.
        </p>
      </PanelSheet>
    );
  }

  const unconfirmed =
    operation && operation.state === 'submission-unknown' ? operation : null;
  const interrupted = unfinished.filter(
    (row) => row.operation_id !== operation?.operation_id,
  );
  return (
    <PanelSheet
      presentation={{ title: 'Invite a new user', onClose }}
      title={`Invite a new user to ${team.name}`}
      step="Step 1 of 2"
      busy={busy}
      glyph={glyph}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            disabled={busy || (needPin && !pin)}
            onClick={() => void create()}
          >
            {busy
              ? 'Creating…'
              : operation?.operation_id
                ? 'Finish creating'
                : 'Create invitation'}
          </Button>
        </>
      }
    >
      {interrupted.map((row) => {
        const note = row.operation_id
          ? invitationLabel(profile, account, row.operation_id)
          : undefined;
        return (
          <Band
            key={row.operation_id}
            label="Invitation not finished"
            action={
              row.state === 'prepared' ? (
                <>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy}
                    onClick={() => void run(() => finish(row))}
                  >
                    Finish creating
                  </Button>
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => void discard(row)}
                  >
                    Discard
                  </Button>
                </>
              ) : (
                <Button
                  size="sm"
                  variant="primary"
                  disabled={busy}
                  onClick={() => void checkStatus(row)}
                >
                  Check status
                </Button>
              )
            }
          >
            {note?.label || 'Untitled invitation'}
            {note ? `, ${relativeTime(note.created)}` : ''} ·{' '}
            {operationStateText(row.state)}.
          </Band>
        );
      })}
      {unconfirmed ? (
        <Notice
          title="Not confirmed"
          actions={
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void checkStatus(unconfirmed)}
            >
              Check status
            </Button>
          }
        >
          <p>
            {serverLabel} did not respond, so this device does not know whether
            the invitation was recorded. Check status asks the server. It does
            not create a second invitation.
          </p>
        </Notice>
      ) : null}
      {errorLine}
      <p>
        You will get an invitation to share. When they use it, their request
        appears under <b>Requests</b> for you to approve.
      </p>
      <Inset className="form">
        <InsetRow label="For">
          <input
            value={label}
            placeholder="Optional note, e.g. “Sam, new contractor”"
            maxLength={80}
            disabled={busy || Boolean(operation)}
            onChange={(e) => setLabel(e.target.value)}
          />
        </InsetRow>
        <InsetRow label="Server" action={<Chip>This team’s server</Chip>}>
          <span className="dim">{serverLabel}</span>
        </InsetRow>
        {pinRow}
      </Inset>
      <SectionLabel>Role when you approve</SectionLabel>
      <Inset>
        <RadioGroup label="Role when you approve">
          <RadioCard
            selected={role === 'admin'}
            disabled={busy || Boolean(operation)}
            onSelect={() => setRole('admin')}
            title="Admin"
            detail="Add and remove members, manage channels."
          />
          <RadioCard
            selected={role === 'member'}
            disabled={busy || Boolean(operation)}
            onSelect={() => setRole('member')}
            title="Member"
            detail="Read and write in channels and files."
          />
        </RadioGroup>
      </Inset>
    </PanelSheet>
  );
}
