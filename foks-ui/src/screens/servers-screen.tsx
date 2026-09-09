/**
 * The Servers section of Settings, displaying configured servers and detailed server state.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import {
  enqueueProfileWork,
  normalizeCommandError,
  sharedServerStatus,
} from '../bridge';
import type {
  Bridge,
  CheckedServer,
  ResetPreview,
  ServerStatusSnapshot,
} from '../bridge';
import {
  Band,
  Button,
  Chip,
  Field,
  Icon,
  Inset,
  InsetRow,
  SectionLabel,
  SheetDialog,
  Toggle,
} from '../components';
import type { Location } from '../location';
import { hue, initials, plural, serverLeaseState, shortId } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import type { Server, TeamStore, World } from '../model';

interface Props {
  world: World;
  bridge: Bridge;
  /** The server the section is open on, or nothing for the list. */
  profile?: string;
  /** The named fixture scene Settings was entered at, captured once there. */
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}

type Sheet = 'add' | 'reset' | 'forget' | null;

const expires = (value: number | null): string => {
  if (value === null) return 'No expiration date';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(date)
    : 'Invalid date';
};
/** Formats expiration timestamps for compact server row displays. */
const expiresShort = (value: number | null): string => {
  if (value === null) return 'no expiration';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
      }).format(date)
    : 'Unknown date';
};
const acceptanceText = (value: CheckedServer['acceptance']): string =>
  value === 'inserted'
    ? 'Server identity pinned'
    : value === 'advanced'
      ? 'Server verification updated'
      : 'Server verification unchanged';

function serverFor(
  world: World,
  profile: string | undefined,
): Server | undefined {
  return profile
    ? world.servers.find((server) => server.id === profile)
    : undefined;
}

/** UI state representing server health and connectivity. */
type ServerUiState =
  'checked' | 'unprobed' | 'lapsed' | 'unavailable' | 'blocked' | 'pending';

const STATE_LABEL: Readonly<Record<ServerUiState, string>> = {
  checked: 'Checked',
  unprobed: 'Never checked',
  lapsed: 'Check-in expired',
  unavailable: 'Status unknown',
  blocked: 'Untrusted',
  pending: 'Checking status',
};

/** Returns true if the server state blocks read and write operations. */
const isLocked = (state: ServerUiState): boolean =>
  state === 'lapsed' || state === 'unavailable' || state === 'blocked';

const markTone = (state: ServerUiState): string =>
  state === 'checked' ? 'ok' : isLocked(state) ? 'bad' : '';

function resolveServerUiState(
  server: Server,
  snapshot: ServerStatusSnapshot | undefined,
  statusFailed: boolean,
  fixtureLapsed: boolean,
  rollback = false,
): ServerUiState {
  if (rollback || server.state === 'blocked') return 'blocked';
  const lease = serverLeaseState(snapshot);
  if (fixtureLapsed || server.state === 'lease-lapsed' || lease === 'lapsed')
    return 'lapsed';
  if (statusFailed) return 'unavailable';
  if (snapshot) {
    if (snapshot.host && lease === 'fresh') return 'checked';
    if (!snapshot.host) return 'unprobed';
    if (lease === 'unavailable') return 'unavailable';
  }
  if (server.state === 'lease-unavailable') return 'unavailable';
  if (server.state === 'never-probed') return 'unprobed';
  return 'pending';
}

function StatusChip({ state }: { state: ServerUiState }): ReactNode {
  return (
    <Chip
      tone={
        state === 'checked'
          ? 'ok'
          : state === 'unprobed' || state === 'pending'
            ? 'default'
            : 'bad'
      }
    >
      {STATE_LABEL[state]}
    </Chip>
  );
}

/** Status indicator icon for a server. */
function ServerMark({ state }: { state: ServerUiState }): ReactNode {
  const locked = isLocked(state);
  return (
    <span className={['smark', markTone(state)].filter(Boolean).join(' ')}>
      <Icon name={locked ? 'alert' : 'server'} />
      {locked ? null : <i className="dot" />}
    </span>
  );
}

/** Returns the location object for the servers section or a specific server profile. */
const servers = (profile?: string): Location => ({
  kind: 'settings',
  section: 'servers',
  ...(profile ? { profile } : {}),
});

export function ServersSection({
  world,
  bridge,
  profile,
  scene,
  onNavigate,
  onRefresh,
  onError,
  onMutationError,
}: Props): ReactNode {
  const [enteredScene] = useState(scene);
  const [statuses, setStatuses] = useState<Map<string, ServerStatusSnapshot>>(
    new Map(),
  );
  const [statusFailures, setStatusFailures] = useState<Set<string>>(new Set());
  const [checked, setChecked] = useState<Map<string, CheckedServer>>(new Map());
  const [sheet, setSheet] = useState<Sheet>(() =>
    enteredScene === 'servers-add'
      ? 'add'
      : enteredScene === 'servers-reset'
        ? 'reset'
        : null,
  );
  const [reset, setReset] = useState<ResetPreview | null>(null);
  const [resetLoading, setResetLoading] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const selected = serverFor(world, profile);
  const seededCheck = useRef(false);
  const rollback = enteredScene === 'servers-rollback';
  const fixtureLapsed = Boolean(
    bridge.fixtureWorld &&
    (enteredScene === 'servers-list' ||
      enteredScene === 'servers-add' ||
      enteredScene === 'servers-lapsed'),
  );

  // Reset preview tokens are invalidated when the window loses focus.
  useEffect(() => {
    const conceal = (): void => {
      setSheet(null);
      setReset(null);
      setResetError(null);
    };
    const concealWhenHidden = (): void => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const rows = new Map<string, ServerStatusSnapshot>();
      const failures = new Set<string>();
      for (const server of world.servers) {
        if (server.state === 'blocked') continue;
        try {
          const status = await sharedServerStatus(bridge, server.id);
          if (status.profile !== server.id)
            throw new Error(
              'describe_server_status returned a different profile.',
            );
          rows.set(server.id, status);
        } catch (error) {
          failures.add(server.id);
          if (alive) onError(error);
        }
      }
      if (alive) {
        setStatuses(rows);
        setStatusFailures(failures);
      }
    })();
    return () => {
      alive = false;
    };
  }, [bridge, onError, world.servers]);

  useEffect(() => {
    if (
      !bridge.fixtureWorld ||
      enteredScene !== 'servers-check' ||
      !selected ||
      seededCheck.current
    )
      return;
    seededCheck.current = true;
    void enqueueProfileWork(bridge, selected.id, () =>
      bridge.checkServer(selected.id),
    )
      .then(async (report) => {
        if (report.profile !== selected.id)
          throw new Error('check_server returned a different profile.');
        setChecked((current) => new Map(current).set(selected.id, report));
        const passive = await enqueueProfileWork(bridge, selected.id, () =>
          bridge.describeServerStatus(selected.id),
        );
        if (passive.profile !== selected.id)
          throw new Error(
            'describe_server_status returned a different profile.',
          );
        setStatuses((current) => new Map(current).set(selected.id, passive));
        setStatusFailures((current) => {
          const next = new Set(current);
          next.delete(selected.id);
          return next;
        });
      })
      .catch((error) => void onMutationError(error));
  }, [bridge, enteredScene, onMutationError, selected]);

  const loadResetPreview = useCallback(() => {
    if (!selected) return;
    setResetLoading(true);
    setResetError(null);
    setReset(null);
    void enqueueProfileWork(bridge, selected.id, () =>
      bridge.describeReset(selected.id),
    )
      .then((preview) => {
        if (preview.profile !== selected.id)
          throw new Error('describe_reset returned a different profile.');
        setReset(preview);
      })
      .catch((error) => {
        setResetError(normalizeCommandError(error).message);
        onError(error);
      })
      .finally(() => setResetLoading(false));
  }, [bridge, onError, selected]);

  useEffect(() => {
    if (sheet !== 'reset' || !selected) return;
    loadResetPreview();
  }, [loadResetPreview, selected, sheet]);

  const check = async (server: Server): Promise<void> => {
    // Prevent duplicate checks from rapid key events and ignore blocked servers.
    if (busy) return;
    if (rollback || server.state === 'blocked') return;
    setBusy(true);
    try {
      const report = await enqueueProfileWork(bridge, server.id, () =>
        bridge.checkServer(server.id),
      );
      if (report.profile !== server.id)
        throw new Error('check_server returned a different profile.');
      setChecked((current) => new Map(current).set(server.id, report));
      toasts.show(
        `Checked ${report.canonicalName}, ${acceptanceText(report.acceptance)}`,
      );
      try {
        const passive = await enqueueProfileWork(bridge, server.id, () =>
          bridge.describeServerStatus(server.id),
        );
        if (passive.profile !== server.id)
          throw new Error(
            'describe_server_status returned a different profile.',
          );
        setStatuses((current) => new Map(current).set(server.id, passive));
        setStatusFailures((current) => {
          const next = new Set(current);
          next.delete(server.id);
          return next;
        });
        await onRefresh(
          `Checked ${report.canonicalName}; refreshed signed server status`,
        );
      } catch (error) {
        setStatusFailures((current) => new Set(current).add(server.id));
        await onMutationError(error);
      }
    } catch (error) {
      await onMutationError(error);
    } finally {
      setBusy(false);
    }
  };

  const currentHost = selected
    ? (checked.get(selected.id) ?? statuses.get(selected.id)?.host ?? null)
    : null;

  // Keyboard shortcut: ⌘R / Ctrl+R triggers a check on the selected server.
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (
        event.defaultPrevented ||
        event.target instanceof HTMLInputElement ||
        event.target instanceof HTMLTextAreaElement
      )
        return;
      if (
        (event.metaKey || event.ctrlKey) &&
        event.key.toLowerCase() === 'r' &&
        selected
      ) {
        event.preventDefault();
        void check(selected);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

  const overlay =
    sheet === 'add' ? (
      <AddServerSheet
        bridge={bridge}
        onClose={() => setSheet(null)}
        onAdded={async (added) => {
          setSheet(null);
          await onRefresh(
            'Server added. Check the server to verify its connection.',
          );
          onNavigate(servers(added));
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : sheet === 'reset' && selected ? (
      <ResetSheet
        server={selected}
        preview={reset}
        resetLoading={resetLoading}
        resetError={resetError}
        onRetryPreview={loadResetPreview}
        bridge={bridge}
        onClose={() => {
          setSheet(null);
          setReset(null);
        }}
        onReset={async () => {
          setSheet(null);
          setReset(null);
          await onRefresh(
            `${selected.name} has been reset. Verify the server before reconnecting.`,
          );
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : sheet === 'forget' && selected ? (
      <ForgetSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onForgot={async () => {
          setSheet(null);
          onNavigate(servers());
          await onRefresh(`Removed ${selected.name}`);
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : null;

  return (
    <>
      {selected ? (
        <ServerBody
          world={world}
          server={selected}
          status={statuses.get(selected.id)}
          statusFailed={statusFailures.has(selected.id)}
          host={currentHost}
          checked={checked.get(selected.id)}
          rollback={rollback}
          fixtureLapsed={fixtureLapsed && selected.id === 'acme'}
          busy={busy}
          onBack={() => onNavigate(servers())}
          onCheck={() => void check(selected)}
          onReset={() => setSheet('reset')}
          onForget={() => setSheet('forget')}
          onCopy={(text) =>
            void bridge
              .copyText(text)
              .then(() => toasts.show('Host ID copied to clipboard'))
              .catch(onError)
          }
          onOpenGroup={(store) => onNavigate({ kind: 'store', ref: store.id })}
        />
      ) : (
        <ServerList
          world={world}
          statuses={statuses}
          statusFailures={statusFailures}
          fixtureLapsed={fixtureLapsed}
          busy={busy}
          onOpen={(next) => onNavigate(servers(next))}
          onCheck={(server) => void check(server)}
          onAdd={() => setSheet('add')}
        />
      )}
      {overlay}
    </>
  );
}

/** Renders summary status metadata for a server row. */
function StatusLine({
  state,
  account,
  groups,
  expiry,
}: {
  state: ServerUiState;
  account?: { username: string };
  groups: string[];
  expiry: number | null;
}): ReactNode {
  const sep = <span className="sep">·</span>;
  if (state === 'checked')
    return (
      <>
        {account ? account.username : 'No account'}
        {sep}
        {groups.length ? plural(groups.length, 'group') : 'No groups'}
        {sep}Valid until {expiresShort(expiry)}
      </>
    );
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
        {sep}Verify connection before use
      </>
    );
  if (state === 'lapsed')
    return (
      <>
        <b>Check-in expired</b>
        {sep}Locked since {expiresShort(expiry)}
      </>
    );
  if (state === 'unavailable')
    return (
      <>
        <b>Check-in status unknown</b>
        {sep}Locked until connection is verified
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
  world,
  server,
  state,
  expiry,
  busy,
  onOpen,
  onCheck,
}: {
  world: World;
  server: Server;
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
}): ReactNode {
  const account = world.accounts.find(
    (item) => item.server === server.id || item.server === server.name,
  );
  const groups = world.stores
    .filter((store) => store.kind === 'team' && store.server === server.id)
    .map((store) => store.name);
  return (
    <InsetRow
      className={['srow', isLocked(state) ? 'crit' : '']
        .filter(Boolean)
        .join(' ')}
      valueClass="srv"
      action={
        <>
          <StatusChip state={state} />
          {state === 'unprobed' ? (
            <Button
              size="sm"
              variant="primary"
              icon="again"
              disabled={busy}
              onClick={() => onCheck(server)}
            >
              Check
            </Button>
          ) : null}
          <Button size="sm" onClick={() => onOpen(server.id)}>
            Open
          </Button>
        </>
      }
    >
      <ServerMark state={state} />
      <span className="t">
        <b>
          <span>{server.name}</span>
          {server.label ? <em>{server.label}</em> : null}
        </b>
        <small>
          <StatusLine
            state={state}
            account={account}
            groups={groups}
            expiry={expiry}
          />
        </small>
      </span>
    </InsetRow>
  );
}

function ServerList({
  world,
  statuses,
  statusFailures,
  fixtureLapsed,
  busy,
  onOpen,
  onCheck,
  onAdd,
}: {
  world: World;
  statuses: Map<string, ServerStatusSnapshot>;
  statusFailures: Set<string>;
  fixtureLapsed: boolean;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
  onAdd: () => void;
}): ReactNode {
  const rows = world.servers.map((server) => {
    const snapshot = statuses.get(server.id);
    const state = resolveServerUiState(
      server,
      snapshot,
      statusFailures.has(server.id),
      fixtureLapsed && server.id === 'acme',
    );
    return { server, state, expiry: snapshot?.leaseExpiresAt ?? null };
  });
  // Servers requiring user attention are displayed at the top of the list.
  const attention = rows.filter((row) => isLocked(row.state));
  const ready = rows.filter((row) => !isLocked(row.state));
  const box = (entries: typeof rows): ReactNode => (
    <Inset className="settings-inset middle">
      {entries.map(({ server, state, expiry }) => (
        <ServerRow
          key={server.id}
          world={world}
          server={server}
          state={state}
          expiry={expiry}
          busy={busy}
          onOpen={onOpen}
          onCheck={onCheck}
        />
      ))}
    </Inset>
  );
  const add = (
    <span className="right">
      <Button size="sm" icon="plus" onClick={onAdd}>
        Add a server…
      </Button>
    </span>
  );
  return (
    <>
      {attention.length ? (
        <>
          <SectionLabel
            className="danger-title"
            action={ready.length ? undefined : add}
          >
            Needs attention
          </SectionLabel>
          {box(attention)}
        </>
      ) : null}
      {ready.length ? (
        <>
          <SectionLabel action={add}>
            {attention.length ? 'Ready' : 'Configured servers'}
          </SectionLabel>
          {box(ready)}
        </>
      ) : null}
      {!rows.length ? (
        <>
          <SectionLabel action={add}>Servers on this Mac</SectionLabel>
          <Inset className="settings-inset">
            <div className="sempty">
              <ServerMark state="unprobed" />
              <b>No servers on this Mac yet</b>
              <p>Add a server using the field above to begin.</p>
            </div>
          </Inset>
        </>
      ) : null}
      <div className="sfoot">
        <Icon name="shield" />
        FOKS verifies the server’s identity against its pinned certificate
        before every operation.
      </div>
    </>
  );
}

/**
 * Displays an alert banner and primary action for servers requiring attention.
 */
function StatusBand({
  state,
  expiry,
  busy,
  onCheck,
  onReset,
}: {
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onCheck: () => void;
  onReset: () => void;
}): ReactNode {
  if (state === 'unprobed')
    return (
      <Band
        severity="info"
        label="Not checked yet."
        action={
          <Button
            size="sm"
            variant="primary"
            icon="again"
            disabled={busy}
            onClick={onCheck}
          >
            Check now
          </Button>
        }
      >
        The address is saved. Check the server to verify its certificate and
        connection.
      </Band>
    );
  if (state === 'lapsed')
    return (
      <Band
        severity="crit"
        label="Check-in expired."
        action={
          <Button
            variant="plain"
            size="sm"
            icon="again"
            disabled={busy}
            onClick={onCheck}
          >
            Check now
          </Button>
        }
      >
        The server connection expired {expires(expiry)}. Check the server to
        renew your check-in.
      </Band>
    );
  if (state === 'unavailable')
    return (
      <Band
        severity="crit"
        label="Check-in status unknown."
        action={
          <Button
            size="sm"
            variant="primary"
            icon="again"
            disabled={busy}
            onClick={onCheck}
          >
            Check now
          </Button>
        }
      >
        Cannot verify the status of this server. This server is locked until
        reconnected.
      </Band>
    );
  if (state === 'blocked')
    return (
      <Band
        severity="crit"
        label="Server identity mismatch"
        action={
          <Button size="sm" variant="danger" onClick={onReset}>
            Reset…
          </Button>
        }
      >
        The server certificate or security history does not match the pinned
        identity on this Mac. Access has been blocked for your security.
      </Band>
    );
  return null;
}

/** A named group on this server, as a chip that opens it. */
function GroupChip({
  store,
  onOpen,
}: {
  store: TeamStore;
  onOpen: () => void;
}): ReactNode {
  return (
    <button type="button" className="gchip" onClick={onOpen}>
      <span className="av team" style={{ background: hue(store.name) }}>
        {initials(store.name)}
      </span>
      {store.name}
    </button>
  );
}

function ServerBody({
  world,
  server,
  status,
  statusFailed,
  host,
  checked,
  rollback,
  fixtureLapsed,
  busy,
  onBack,
  onCheck,
  onReset,
  onForget,
  onCopy,
  onOpenGroup,
}: {
  world: World;
  server: Server;
  status?: ServerStatusSnapshot;
  statusFailed: boolean;
  host: ServerStatusSnapshot['host'];
  checked?: CheckedServer;
  rollback: boolean;
  fixtureLapsed: boolean;
  busy: boolean;
  onBack: () => void;
  onCheck: () => void;
  onReset: () => void;
  onForget: () => void;
  onCopy: (text: string) => void;
  onOpenGroup: (store: TeamStore) => void;
}): ReactNode {
  const state = resolveServerUiState(
    server,
    status,
    statusFailed,
    fixtureLapsed,
    rollback,
  );
  const locked = isLocked(state);
  const blocked = state === 'blocked';
  const expiry = status?.leaseExpiresAt ?? null;
  const account = world.accounts.find(
    (item) => item.server === server.id || item.server === server.name,
  );
  const groups = world.stores.filter(
    (store): store is TeamStore =>
      store.kind === 'team' && store.server === server.id,
  );
  const named = groups.filter((store) => store.team_kind === 'named');
  const subtitle = server.label
    ? `${server.label}${account && !locked ? ` · signed in as ${account.username}` : ''}`
    : account && !locked
      ? `signed in as ${account.username}`
      : '';
  const hasHost = Boolean(host) && state !== 'unavailable';

  return (
    <>
      <button type="button" className="crumb" onClick={onBack}>
        <Icon name="back" className="ic" />
        Servers
      </button>
      <div className="shead">
        <ServerMark state={state} />
        <span className="t">
          <b>{server.name}</b>
          <small>{subtitle}</small>
        </span>
        <StatusChip state={state} />
      </div>
      <StatusBand
        state={state}
        expiry={expiry}
        busy={busy}
        onCheck={onCheck}
        onReset={onReset}
      />

      <SectionLabel>Check-in</SectionLabel>
      <Inset className="settings-inset middle">
        {state === 'checked' ? (
          <>
            <InsetRow
              label="Status"
              action={
                <Button
                  size="sm"
                  icon="again"
                  disabled={busy}
                  onClick={onCheck}
                >
                  Check
                </Button>
              }
            >
              {checked
                ? 'Checked just now. History unchanged.'
                : 'Identity pinned on this Mac.'}
              <small>History is checked before every operation.</small>
            </InsetRow>
            <InsetRow label="Expires">
              {expires(expiry)}
              <small>Automatically renewed in the background.</small>
            </InsetRow>
          </>
        ) : state === 'pending' ? (
          <InsetRow label="Status">Verifying server status…</InsetRow>
        ) : state === 'unprobed' ? (
          <InsetRow label="Expires">
            <span className="stopped">Not verified yet</span>
          </InsetRow>
        ) : state === 'lapsed' ? (
          <InsetRow label="Expired">
            <b className="danger-title">{expires(expiry)}</b>
            <small>Automatically renewed in the background.</small>
          </InsetRow>
        ) : state === 'unavailable' ? (
          <InsetRow label="Expires">
            <span className="stopped">Unknown</span>
          </InsetRow>
        ) : (
          <InsetRow label="Expires">
            <span className="stopped">Not read while locked</span>
          </InsetRow>
        )}
      </Inset>

      <SectionLabel>On this server</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow label="You">
          {locked ? (
            <span className="stopped">Hidden while locked</span>
          ) : account ? (
            <>
              <b>{account.username}</b>
              <small>Local alias: {account.alias}</small>
            </>
          ) : (
            <>
              No account on this server<small>Read-only access available</small>
            </>
          )}
        </InsetRow>
        <InsetRow label="Groups">
          {locked ? (
            <span className="stopped">
              {groups.length
                ? `${groups.map((store) => store.name).join(', ')} — locked`
                : 'Hidden while locked'}
            </span>
          ) : named.length ? (
            <span className="gchips">
              {named.map((store) => (
                <GroupChip
                  key={store.id}
                  store={store}
                  onOpen={() => onOpenGroup(store)}
                />
              ))}
            </span>
          ) : (
            'No groups configured'
          )}
        </InsetRow>
      </Inset>

      <SectionLabel>Identity</SectionLabel>
      {hasHost && host ? (
        <>
          <Inset className="settings-inset middle">
            <InsetRow label="Address">{server.name}</InsetRow>
            <InsetRow
              label="Host ID"
              action={
                <Button
                  size="sm"
                  icon="copy"
                  disabled={blocked}
                  onClick={() => onCopy(host.hostId)}
                >
                  Copy
                </Button>
              }
            >
              <span className="hostid" title={host.hostId}>
                <code>{shortId(host.hostId, 8)}</code>
              </span>
              <small>
                Hover to view the full ID. Click Copy to copy the full value.
              </small>
            </InsetRow>
            <InsetRow label="Audit log">
              Verified · Checkpoint{' '}
              {new Date(host.epoch * 1000).toLocaleDateString()}
            </InsetRow>
          </Inset>
          <Toggle
            label="View diagnostic response"
            open={rollback || undefined}
            disabled={rollback}
          >
            <pre>
              {JSON.stringify(
                {
                  profile: status?.profile ?? server.id,
                  configuredProbe: status?.configuredProbe ?? server.name,
                  host,
                  leaseRequired: status?.leaseRequired ?? null,
                  leaseExpiresAt: expiry,
                },
                null,
                2,
              )}
            </pre>
          </Toggle>
        </>
      ) : (
        <Inset className="settings-inset middle">
          <InsetRow label="Address">{server.name}</InsetRow>
          <InsetRow label="Host ID">
            <span className="stopped">
              {state === 'unprobed'
                ? 'Discovered upon first verification'
                : state === 'pending'
                  ? 'Reading signed status…'
                  : 'Hidden while locked'}
            </span>
          </InsetRow>
        </Inset>
      )}

      <SectionLabel className="danger-title">Manage Server Data</SectionLabel>
      <Inset className="settings-inset middle danger-box">
        <InsetRow
          className="dangerrow"
          label="Forget this server"
          action={
            <Button
              size="sm"
              icon="trash"
              disabled={blocked}
              onClick={onForget}
            >
              Forget…
            </Button>
          }
        >
          <small>Removes this server from this Mac.</small>
        </InsetRow>
        <InsetRow
          className="dangerrow"
          label="Erase local credentials and reset trust"
          action={
            <Button size="sm" variant="danger" onClick={onReset}>
              Erase and reset…
            </Button>
          }
        >
          <small>
            Deletes this server's local account keys, trust history, cache and
            unfinished operations. A separate recovery method is required.
          </small>
        </InsetRow>
      </Inset>
    </>
  );
}

function SheetFrame({
  title,
  subtitle,
  children,
  footer,
  onClose,
  danger = false,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  danger?: boolean;
}): ReactNode {
  return (
    <SheetDialog
      width="wide"
      danger={danger}
      onClose={onClose}
      title={title}
      subtitle={subtitle}
      footer={footer}
      glyph={
        <span className={`server-mark ${danger ? 'danger' : ''}`}>
          <Icon name={danger ? 'trash' : 'server'} />
        </span>
      }
    >
      {children}
    </SheetDialog>
  );
}

function AddServerSheet({
  bridge,
  onClose,
  onAdded,
  onError,
}: {
  bridge: Bridge;
  onClose: () => void;
  onAdded: (profile: string) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [profile, setProfile] = useState('partner');
  const [probe, setProbe] = useState('foks.partner.dev');
  const [busy, setBusy] = useState(false);
  const cleanProbe = probe.trim();
  const valid =
    /^[A-Za-z0-9_-]{1,64}$/.test(profile) &&
    cleanProbe.length > 0 &&
    !cleanProbe.includes('\0') &&
    !cleanProbe.includes('\n') &&
    !cleanProbe.includes('\r') &&
    new TextEncoder().encode(cleanProbe).length <= 2048;
  return (
    <SheetFrame
      title="Add a server"
      subtitle="Enter server connection details to verify and connect"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .addServer(profile, cleanProbe)
                .then((added) => {
                  if (added.profile !== profile)
                    throw new Error('add_server returned a different profile.');
                  return onAdded(added.profile);
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Add server
          </Button>
        </>
      }
    >
      <p>
        Adding a server saves its profile and address on this Mac. Verify the
        server after adding it to confirm its identity and establish the
        connection.
      </p>
      <Inset>
        <Field label="Profile" value={profile} onChange={setProfile} />
        <Field label="Address" value={probe} onChange={setProbe} />
      </Inset>
    </SheetFrame>
  );
}

function ResetSheet({
  server,
  preview,
  resetLoading,
  resetError,
  onRetryPreview,
  bridge,
  onClose,
  onReset,
  onError,
}: {
  server: Server;
  preview: ResetPreview | null;
  resetLoading: boolean;
  resetError: string | null;
  onRetryPreview: () => void;
  bridge: Bridge;
  onClose: () => void;
  onReset: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const [available, setAvailable] = useState(false);
  const token = useRef<string | null>(null);
  useEffect(() => {
    token.current = preview?.token ?? null;
    setAvailable(Boolean(preview?.token));
  }, [preview?.token]);
  return (
    <SheetFrame
      title={`Erase local credentials for ${server.name}?`}
      subtitle="Delete local account keys and reset server trust on this device"
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      danger
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={
              confirmation !== server.id || !preview || !available || busy
            }
            onClick={() => {
              const once = token.current;
              token.current = null;
              setAvailable(false);
              if (!once) return;
              setBusy(true);
              void bridge
                .resetServer(server.id, confirmation, once)
                .then(onReset)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Erase credentials and reset
          </Button>
        </>
      }
    >
      <p>
        This permanently deletes local account keys for this server. Server
        data is not deleted, but you can permanently lose access to it without
        another enrolled device, a saved recovery phrase for an enrolled backup,
        or a usable external backup of your local state. Your account passphrase
        alone cannot restore the deleted keys.
      </p>
      <Inset>
        <InsetRow label="Removed">
          Local account keys and credentials, server trust history, local cache,
          and pending operations.
        </InsetRow>
        <InsetRow label="Unrecoverable">
          Pending local changes not yet uploaded to the server will be lost.
        </InsetRow>
        <InsetRow label="Recovery required">
          Verify the server again and recover or pair an account before using
          it on this device. Saving a recovery phrase here is not part of this
          operation.
        </InsetRow>
        <InsetRow label="Unaffected">
          Other configured servers and their local data.
        </InsetRow>
      </Inset>
      {preview ? (
        <>
          <SectionLabel>Discarded pending operations</SectionLabel>
          <Inset>
            {preview.resumables.length ? (
              preview.resumables.map((row, index) => (
                <InsetRow
                  key={`${row.kind}-${row.alias}-${index}`}
                  label={row.kind}
                >
                  {row.alias}
                  {row.target ? ` · ${row.target}` : ''}
                </InsetRow>
              ))
            ) : (
              <InsetRow label="None">No resumable operations found.</InsetRow>
            )}
          </Inset>
          <SectionLabel>Local credentials and data to erase</SectionLabel>
          <Inset>
            {preview.artifacts.length ? (
              preview.artifacts.map((row) => (
                <InsetRow key={row.kind} label={row.kind}>
                  {row.entries} entries · {row.bytes.toLocaleString()} bytes
                </InsetRow>
              ))
            ) : (
              <InsetRow label="None">No local data to erase.</InsetRow>
            )}
          </Inset>
          <p className="hint">
            This reset confirmation expires in {preview.expiresInSeconds}{' '}
            seconds.
            {available
              ? ''
              : ' Reopen this dialog to generate a new confirmation.'}
          </p>
        </>
      ) : resetError ? (
        <div style={{ margin: '16px 0' }}>
          <p
            className="hint"
            style={{ color: 'var(--red, #e5484d)', marginBottom: '8px' }}
          >
            Failed to load reset preview: {resetError}
          </p>
          <Button disabled={resetLoading} onClick={onRetryPreview}>
            {resetLoading ? 'Retrying…' : 'Retry loading preview'}
          </Button>
        </div>
      ) : (
        <p>Loading reset preview…</p>
      )}
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`Type "${server.id}" to confirm`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}

function ForgetSheet({
  server,
  bridge,
  onClose,
  onForgot,
  onError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onForgot: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <SheetFrame
      title={`Forget ${server.name}?`}
      subtitle="Remove all local keys and stored data for this server"
      onClose={onClose}
      danger
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={confirmation !== server.id || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .forgetServer(server.id, confirmation)
                .then((forgotten) => {
                  if (forgotten.profile !== server.id)
                    throw new Error(
                      'forget_server returned a different profile.',
                    );
                  return onForgot();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Forget server
          </Button>
        </>
      }
    >
      <p>
        Your data on the server will not be deleted. Type the server profile
        name to confirm.
      </p>
      <Inset>
        <InsetRow label="Removed">
          All credentials for this server, pinned host identity, connection
          history, and cached data.
        </InsetRow>
        <InsetRow label="Warning">
          Accounts without a backup recovery phrase or another paired device
          cannot be accessed again.
        </InsetRow>
        <InsetRow label="Unaffected">
          Other configured servers and their local data.
        </InsetRow>
      </Inset>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`Type "${server.id}" to confirm`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}
