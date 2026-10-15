/** The server list on Settings › Account: one row per server on this device. */

import type { ReactNode } from 'react';
import type { ServerStatusSnapshot } from '../../bridge';
import { Button, Inset, InsetRow, SectionLabel } from '../../components';
import { CollectionStatus } from '../../components/collection-status';
import { inventoryReadiness, serverDisplayLabelOrLoading } from '../../model';
import type { AgentSnapshot, Server } from '../../model';
import {
  expiresShort,
  isLocked,
  leaseExpiry,
  resolveServerUiState,
} from './server-model';
import type { ServerUiState } from './server-model';
import { ServerMark, StatusChip } from './server-status';

/** Explains the attention state of a server row. */
function StatusLine({
  state,
  expiry,
}: {
  state: ServerUiState;
  expiry: number | null;
}): ReactNode {
  const sep = <span className="sep">·</span>;
  if (state === 'pending')
    return (
      <>
        <b>Connecting</b>
        {sep}Waiting for server response
      </>
    );
  if (state === 'unprobed')
    return (
      <>
        <b>Not verified</b>
        {sep}Verify this server before use
      </>
    );
  if (state === 'lapsed')
    return (
      <>
        <b>Check-in expired</b>
        {sep}Locked since {expiresShort(expiry)}
      </>
    );
  if (state === 'recovery-required')
    return (
      <>
        <b>Security state missing</b>
        {sep}Restore saved trust before reconnecting
      </>
    );
  if (state === 'unavailable')
    return (
      <>
        <b>Check-in status unknown</b>
        {sep}Locked until server status is verified
      </>
    );
  if (state === 'schema')
    return (
      <>
        <b>Schema incompatible</b>
        {sep}Locked until a compatible client can inspect it
      </>
    );
  if (state === 'import-verification')
    return (
      <>
        <b>Verification required</b>
        {sep}Check the imported profile online before use
      </>
    );
  return (
    <>
      <b>Identity changed</b>
      {sep}Locked. The server identity does not match the pinned certificate.
    </>
  );
}

/**
 * Renders an individual server row in the settings server list.
 */
function ServerRow({
  server,
  state,
  expiry,
  busy,
  onOpen,
  onCheck,
}: {
  server: Server;
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
}): ReactNode {
  return (
    <InsetRow
      className={['srow', isLocked(state) ? 'crit' : '']
        .filter(Boolean)
        .join(' ')}
      valueClass="srv"
      action={
        <>
          <StatusChip state={state} />
          {/* A never-checked server and a lapsed one are equally stuck, and
              the command is the same one, so both rows offer it. */}
          {state === 'unprobed' || state === 'lapsed' ? (
            <Button
              size="sm"
              variant={state === 'unprobed' ? 'primary' : 'plain'}
              icon="refresh"
              disabled={busy}
              onClick={() => onCheck(server)}
            >
              Check
            </Button>
          ) : null}
          <Button
            size="sm"
            className="account-fact-link"
            onClick={() => onOpen(server.profileName)}
          >
            Manage ›
          </Button>
        </>
      }
    >
      <ServerMark state={state} />
      <span className="t">
        <b>
          <span>{serverDisplayLabelOrLoading(server)}</span>
        </b>
        {server.configuredEndpoint !== serverDisplayLabelOrLoading(server) && (
          <small>{server.configuredEndpoint}</small>
        )}
        {state === 'checked' ? null : (
          <small>
            <StatusLine state={state} expiry={expiry} />
          </small>
        )}
      </span>
    </InsetRow>
  );
}

export function ServerList({
  snapshot: agentSnapshot,
  statuses,
  busy,
  onOpen,
  onCheck,
  onAdd,
}: {
  snapshot: AgentSnapshot;
  /** Signed statuses this page checked itself, by profile; see `leaseExpiry`. */
  statuses: Map<string, ServerStatusSnapshot>;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
  onAdd: () => void;
}): ReactNode {
  const rows = agentSnapshot.servers.map((server) => ({
    server,
    state: resolveServerUiState(agentSnapshot, server),
    expiry: leaseExpiry(server, statuses.get(server.profileName)),
  }));
  // Servers requiring user attention stay first, but all configured servers
  // share one group on the Account page.
  const needsAttention = (row: (typeof rows)[number]): boolean =>
    isLocked(row.state) || row.state === 'unprobed';
  const attention = rows.filter(needsAttention);
  const ready = rows.filter((row) => !needsAttention(row));
  const ordered = [...attention, ...ready];
  const add = (
    <Button size="sm" icon="plus" onClick={onAdd}>
      Add a server…
    </Button>
  );
  return (
    <>
      <SectionLabel className="server-list-label" action={add}>
        Servers
      </SectionLabel>
      <Inset className="settings-inset middle">
        {ordered.map(({ server, state, expiry }) => (
          <ServerRow
            key={server.profileName}
            server={server}
            state={state}
            expiry={expiry}
            busy={busy}
            onOpen={onOpen}
            onCheck={onCheck}
          />
        ))}
        {!rows.length &&
        inventoryReadiness(agentSnapshot, 'profiles') !== 'ready' ? (
          <CollectionStatus
            state={inventoryReadiness(agentSnapshot, 'profiles')}
            label="servers"
          />
        ) : !rows.length ? (
          <div className="sempty">
            <ServerMark state="unprobed" />
            <b>No servers on this device yet</b>
            <p>Add a server using the button above to begin.</p>
          </div>
        ) : null}
      </Inset>
    </>
  );
}
