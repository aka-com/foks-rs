import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useDeviceCache } from '../device-cache';
import type { InvitationAction, InvitationRow } from '../invitation-contract';
import {
  forgetInvitationLabel,
  invitationLabel,
  relativeTime,
} from '../invitation-labels';
import {
  grantRoleName,
  hardwareUnlockRequested,
  invitationRow,
  invitationRows,
  operationStateText,
  rowRole,
  settleInvitationWrite,
  unfinishedOperation,
} from '../invitation-writes';
import type { TeamStore } from '../model';
import { useMetadataRepository } from '../query-hooks';
import {
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  PanelSheet,
  SectionLabel,
  SegmentedControl,
} from './index';

type View = 'attention' | 'issued';

export function InvitationActivitySheet({
  bridge,
  team,
  onClose,
  onComplete,
}: {
  bridge: Bridge;
  team: TeamStore;
  onClose: () => void;
  onComplete?: () => Promise<void> | void;
}): ReactNode {
  const profile = team.server;
  const account = team.account;
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
  const [view, setView] = useState<View>('attention');
  const [operations, setOperations] = useState<InvitationRow[]>([]);
  const [approvals, setApprovals] = useState<InvitationRow[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [pin, setPin] = useState('');
  const [needPin, setNeedPin] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const load = useCallback(async (): Promise<void> => {
    const [list, pending] = await Promise.all([
      bridge.invitation(profile, account, { action: 'list' }, null),
      bridge.invitation(
        profile,
        account,
        { action: 'pending-approvals', team_alias: team.alias },
        null,
      ),
    ]);
    if (!live.current) return;
    setOperations(
      invitationRows(list).filter(
        (row) =>
          row.team_id === team.team_id_hex &&
          row.operation_id &&
          row.state !== 'cancelled',
      ),
    );
    setApprovals(
      invitationRows(pending).filter(
        (row) => row.request_id && row.state !== 'complete',
      ),
    );
    setLoaded(true);
  }, [bridge, profile, account, team.alias, team.team_id_hex]);

  useEffect(() => {
    setBusy(true);
    load()
      .catch((failure: unknown) => {
        if (live.current) setError(normalizeCommandError(failure).message);
      })
      .finally(() => {
        if (live.current) setBusy(false);
      });
  }, [load]);

  const call = (action: InvitationAction) =>
    bridge
      .invitation(profile, account, action, pin || null)
      .then(invitationRow);

  const run = async (
    work: () => Promise<void>,
    options: { write?: boolean } = { write: true },
  ): Promise<void> => {
    setBusy(true);
    setError(null);
    setStatus(null);
    setCopied(null);
    try {
      await work();
    } catch (failure) {
      if (!live.current) return;
      if (hardwareUnlockRequested(failure)) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to continue.');
      } else setError(normalizeCommandError(failure).message);
    } finally {
      if (live.current) setBusy(false);
      if (options.write) settleInvitationWrite(queries, profile, account);
    }
  };

  const absorb = (current: InvitationRow): boolean => {
    if (current.hardware_required) {
      setNeedPin(true);
      setError('Enter the PIN for your security key to continue.');
      return false;
    }
    setPin('');
    setNeedPin(false);
    setOperations((rows) =>
      rows.map((row) =>
        row.operation_id === current.operation_id ? current : row,
      ),
    );
    return true;
  };

  const finish = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      const current = await call({
        action: 'attempt',
        operation_id: row.operation_id,
      });
      if (!live.current) return;
      if (absorb(current) && current.state === 'complete') {
        setStatus('Invitation created.');
        await onComplete?.();
      }
    });

  const check = (row: InvitationRow) =>
    run(
      async () => {
        if (!row.operation_id) return;
        const current = await call({
          action: 'status',
          operation_id: row.operation_id,
        });
        if (!live.current) return;
        if (absorb(current) && current.state === 'complete')
          setStatus('Invitation confirmed.');
      },
      { write: false },
    );

  const discard = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      await call({ action: 'cancel', operation_id: row.operation_id });
      forgetInvitationLabel(profile, account, row.operation_id);
      if (!live.current) return;
      setOperations((rows) =>
        rows.filter((r) => r.operation_id !== row.operation_id),
      );
      setStatus('Unfinished invitation discarded.');
    });

  const resumeApproval = (row: InvitationRow) =>
    run(async () => {
      if (!row.request_id || !row.role) return;
      const current = await call(
        row.remote && row.source_profile
          ? {
              action: 'approve-remote',
              remote_profile: row.source_profile,
              team_alias: team.alias,
              request_id: row.request_id,
              role: rowRole(row.role),
            }
          : {
              action: 'approve',
              team_alias: team.alias,
              request_id: row.request_id,
              role: rowRole(row.role),
            },
      );
      if (!live.current) return;
      if (current.hardware_required) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to continue.');
        return;
      }
      setPin('');
      setNeedPin(false);
      if (current.state === 'complete') {
        setApprovals((rows) =>
          rows.filter((r) => r.request_id !== row.request_id),
        );
        setStatus('Approval finished.');
        await onComplete?.();
      }
    });

  const copyIssued = (row: InvitationRow) =>
    run(
      async () => {
        if (!row.operation_id) return;
        const current = row.invite
          ? row
          : await call({ action: 'status', operation_id: row.operation_id });
        if (!live.current) return;
        if (!absorb(current)) return;
        if (!current.invite)
          throw new Error('The invitation token is not available.');
        await bridge.copyText(current.invite);
        if (live.current) setCopied(row.operation_id);
      },
      { write: false },
    );

  const attention = [...operations.filter(unfinishedOperation), ...approvals];
  const issued = operations.filter((row) => row.state === 'complete');

  const noteOf = (row: InvitationRow) =>
    row.operation_id
      ? invitationLabel(profile, account, row.operation_id)
      : undefined;

  const titleOf = (row: InvitationRow): string =>
    noteOf(row)?.label || 'Untitled invitation';

  const operationRow = (row: InvitationRow): ReactNode => {
    const note = noteOf(row);
    const when = note ? relativeTime(note.created) : null;
    return (
      <article key={row.operation_id} className="op">
        <p>
          <b>{titleOf(row)}</b>
        </p>
        <p className="fn">
          {operationStateText(row.state)}
          {when ? ` · ${when}` : ''}
          {note ? ` · ${grantRoleName(note.role)} on approval` : ''}
        </p>
        <div className="btns">
          {row.state === 'prepared' ? (
            <>
              <Button
                size="sm"
                variant="primary"
                disabled={busy}
                onClick={() => void finish(row)}
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
              onClick={() => void check(row)}
            >
              Check status
            </Button>
          )}
        </div>
        {row.state === 'submission-unknown' ? (
          <p className="fn">
            Check status does not create a second invitation.
          </p>
        ) : null}
      </article>
    );
  };

  const approvalRow = (row: InvitationRow): ReactNode => (
    <article key={row.request_id} className="op">
      <p>
        <b>{row.username ?? 'Unknown user'}</b>{' '}
        <Chip tone="warn">Action required</Chip>
      </p>
      <p className="fn">
        Approving
        {row.role
          ? ` as ${grantRoleName(row.role.kind === 1 ? 'member' : row.role.kind === 2 ? 'admin' : 'owner')}`
          : ''}{' '}
        was interrupted.
      </p>
      <div className="btns">
        <Button
          size="sm"
          variant="primary"
          disabled={busy || !row.role}
          onClick={() => void resumeApproval(row)}
        >
          Resume approval
        </Button>
      </div>
    </article>
  );

  const issuedRow = (row: InvitationRow): ReactNode => {
    const note = noteOf(row);
    return (
      <article key={row.operation_id} className="op">
        <p>
          <b>{titleOf(row)}</b>
        </p>
        <p className="fn">
          {note
            ? `Created ${relativeTime(note.created)} · ${grantRoleName(note.role)} on approval`
            : 'Ready to share'}
        </p>
        <div className="btns">
          <Button
            size="sm"
            icon={copied === row.operation_id ? 'check' : 'copy'}
            aria-live="polite"
            disabled={busy}
            onClick={() => void copyIssued(row)}
          >
            {copied === row.operation_id ? 'Copied' : 'Copy'}
          </Button>
        </div>
      </article>
    );
  };

  return (
    <PanelSheet
      presentation={{ title: 'Invitation activity', onClose }}
      title={`Invitation activity · ${team.name}`}
      busy={busy}
      glyph={
        <span className="kico invite">
          <Icon name="door" />
        </span>
      }
      footer={
        <>
          <Button
            icon="refresh"
            disabled={busy}
            onClick={() => void run(load, { write: false })}
          >
            Refresh
          </Button>
          <span className="spacer" />
          <Button disabled={busy} onClick={onClose}>
            Close
          </Button>
        </>
      }
    >
      <SegmentedControl
        label="Invitation activity view"
        value={view}
        items={[
          {
            id: 'attention' as const,
            label: attention.length
              ? `Needs attention (${attention.length})`
              : 'Needs attention',
          },
          {
            id: 'issued' as const,
            label: issued.length ? `Issued (${issued.length})` : 'Issued',
          },
        ]}
        onChange={setView}
      />
      {error ? (
        <p role="alert" className="crit">
          {error}
        </p>
      ) : null}
      {status ? (
        <p role="status" className="fn">
          {status}
        </p>
      ) : null}
      {needPin ? (
        <Inset className="form">
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
        </Inset>
      ) : null}
      {view === 'attention' ? (
        loaded && !attention.length ? (
          <div className="callout">
            <span className="kico neutral">
              <Icon name="check" />
            </span>
            <span className="t">
              <b>No in-progress invitations.</b>
            </span>
          </div>
        ) : (
          <>
            {operations.filter(unfinishedOperation).map(operationRow)}
            {approvals.map(approvalRow)}
          </>
        )
      ) : loaded && !issued.length ? (
        <div className="callout">
          <span className="kico neutral">
            <Icon name="door" />
          </span>
          <span className="t">
            <b>No invitations yet.</b>
          </span>
        </div>
      ) : (
        <>
          <SectionLabel>Issued</SectionLabel>
          {issued.map(issuedRow)}
          <p className="fn">Issued invitations can be copied again here.</p>
        </>
      )}
    </PanelSheet>
  );
}
