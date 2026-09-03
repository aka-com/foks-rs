/**
 * The Servers section of Settings.
 *
 * A list of every server this Mac knows, then one server at a time. Servers
 * used to be a page of its own; it is drawn the way the other Settings
 * sections are now — a `SectionLabel` and a `settings-inset` per topic, small
 * buttons in the row's action slot, and a `Band` for the state that stops
 * work — so the controls are the same ones with less furniture around them.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { enqueueProfileWork, sharedServerStatus } from '../bridge';
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
}

type Sheet = 'add' | 'reset' | 'forget' | null;

const expires = (value: number | null): string => {
  if (value === null) return 'No signed expiry is available';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(date)
    : `Unix time ${value} seconds`;
};
/** The row's compact form: "Sep 8, 9:14 AM" — the year is noise in a list. */
const expiresShort = (value: number | null): string => {
  if (value === null) return 'no signed expiry';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
      }).format(date)
    : `Unix time ${value}`;
};
const acceptanceText = (value: CheckedServer['acceptance']): string =>
  value === 'inserted'
    ? 'New pin'
    : value === 'advanced'
      ? 'Pinned history advanced safely'
      : 'Pinned history unchanged';

function serverFor(
  world: World,
  profile: string | undefined,
): Server | undefined {
  return profile
    ? world.servers.find((server) => server.id === profile)
    : undefined;
}

/** The five states a server can be in, and the one word each is called. */
type ServerUiState =
  'checked' | 'unprobed' | 'lapsed' | 'unavailable' | 'blocked' | 'pending';

const STATE_LABEL: Readonly<Record<ServerUiState, string>> = {
  checked: 'Checked',
  unprobed: 'Never checked',
  lapsed: 'Check-in expired',
  unavailable: 'Status unknown',
  blocked: 'History changed',
  pending: 'Reading status',
};

/** A stopped server is "locked": no reads, whatever stopped it. */
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

/** The server mark, with the dot that carries the state's colour. */
function ServerMark({ state }: { state: ServerUiState }): ReactNode {
  const locked = isLocked(state);
  return (
    <span className={['smark', markTone(state)].filter(Boolean).join(' ')}>
      <Icon name={locked ? 'alert' : 'server'} />
      {locked ? null : <i className="dot" />}
    </span>
  );
}

/** Where the section lives, with or without a server open. */
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

  // The Reset sheet holds a one-shot preview token. Like the other Settings
  // sheets, it does not outlive the window losing focus.
  useEffect(() => {
    const conceal = (): void => {
      setSheet(null);
      setReset(null);
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
      .catch(onError);
  }, [bridge, enteredScene, onError, selected]);

  useEffect(() => {
    if (sheet !== 'reset' || !selected) return;
    let alive = true;
    setReset(null);
    void enqueueProfileWork(bridge, selected.id, () =>
      bridge.describeReset(selected.id),
    )
      .then((preview) => {
        if (preview.profile !== selected.id)
          throw new Error('describe_reset returned a different profile.');
        if (alive) setReset(preview);
      })
      .catch(onError);
    return () => {
      alive = false;
    };
  }, [bridge, onError, selected, sheet]);

  const check = async (server: Server): Promise<void> => {
    // Guarded here rather than at each caller, because ⌘R reached this with
    // neither the busy flag nor the page's rule: a held key fired a request
    // per repeat, and it would check a blocked or lapsed server the page
    // deliberately offers no Check for.
    if (busy) return;
    const lease = serverLeaseState(statuses.get(server.id));
    const lapsed =
      server.state === 'lease-lapsed' ||
      lease === 'lapsed' ||
      (fixtureLapsed && server.id === 'acme');
    if (lapsed || rollback || server.state === 'blocked') return;
    setBusy(true);
    try {
      const report = await enqueueProfileWork(bridge, server.id, () =>
        bridge.checkServer(server.id),
      );
      if (report.profile !== server.id)
        throw new Error('check_server returned a different profile.');
      setChecked((current) => new Map(current).set(server.id, report));
      toasts.show(
        `Checked ${report.canonicalName} — ${acceptanceText(report.acceptance)}`,
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
        onError(error);
      }
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  };

  const currentHost = selected
    ? (checked.get(selected.id) ?? statuses.get(selected.id)?.host ?? null)
    : null;

  // ⌘R checks the open server. ⌘N and the arrow walk the old page had are
  // gone: inside Settings ⌘N would collide with the other sections' sheets,
  // and an inset of rows is not a selection list.
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
            'Server added — check it before trusting anything on it',
          );
          onNavigate(servers(added));
        }}
        onError={onError}
      />
    ) : sheet === 'reset' && selected ? (
      <ResetSheet
        server={selected}
        preview={reset}
        bridge={bridge}
        onClose={() => {
          setSheet(null);
          setReset(null);
        }}
        onReset={async () => {
          setSheet(null);
          setReset(null);
          await onRefresh(
            `Reset ${selected.name} — check it again before using it`,
          );
        }}
        onError={onError}
      />
    ) : sheet === 'forget' && selected ? (
      <ForgetSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onForgot={async () => {
          setSheet(null);
          onNavigate(servers());
          await onRefresh(`Forgot ${selected.name} on this Mac`);
        }}
        onError={onError}
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
              .then(() => toasts.show('Copied the full ID'))
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

/** One line per row: the state in bold, then the single fact that matters. */
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
        {account ? account.username : 'no account'}
        {sep}
        {groups.length ? plural(groups.length, 'group') : 'no groups'}
        {sep}check-in until {expiresShort(expiry)}
      </>
    );
  if (state === 'pending')
    return (
      <>
        <b>Reading status</b>
        {sep}Waiting for a signed check-in
      </>
    );
  if (state === 'unprobed')
    return (
      <>
        <b>Never checked</b>
        {sep}Check it before use
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
        {sep}Locked until the agent gets one
      </>
    );
  return (
    <>
      <b>History changed</b>
      {sep}Locked. The server no longer matches what this Mac pinned
    </>
  );
}

/**
 * A server as a settings row: mark, name and one status line, then the state
 * chip and Open. A never-checked server carries Check in the row, so the
 * required next step is one click from the list.
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
  // A broken server is the only thing in this list worth acting on, so it
  // leads, under its own label.
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
            {attention.length ? 'Ready' : 'Servers on this Mac'}
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
              <b>No servers on this Mac yet</b>Add one, then check it to pin its
              identity here.
            </div>
          </Inset>
        </>
      ) : null}
      <div className="sfoot">
        <Icon name="shield" />
        Before every operation, FOKS checks the server’s history against what
        this Mac pinned.
      </div>
    </>
  );
}

/**
 * The band that says what stops work, and the one action that answers it.
 * A checked server has nothing to say here; its Check lives in the Check-in
 * row instead, so the verb is never offered twice.
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
        The address is saved. Check the server to pin its identity on this Mac.
      </Band>
    );
  if (state === 'lapsed')
    // Check is not offered: the agent renews the check-in, not this button.
    return (
      <Band severity="crit" label="Check-in expired.">
        The server’s check-in expired {expires(expiry)}. Until the agent renews
        it, this server is locked.
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
        The agent has no signed check-in for this server. Until it gets one,
        this server is locked.
      </Band>
    );
  if (state === 'blocked')
    return (
      <Band
        severity="crit"
        label="History changed."
        action={
          <Button size="sm" variant="danger" onClick={onReset}>
            Reset…
          </Button>
        }
      >
        The server’s history no longer matches what this Mac pinned. This server
        is locked.
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
      : 'No label';
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
                ? 'Last checked: now. Trust history unchanged.'
                : 'Identity pinned on this Mac.'}
              <small>History is checked before every operation.</small>
            </InsetRow>
            <InsetRow label="Expires">
              {expires(expiry)}
              <small>Renewed by the agent.</small>
            </InsetRow>
          </>
        ) : state === 'pending' ? (
          <InsetRow label="Status">Reading signed status…</InsetRow>
        ) : state === 'unprobed' ? (
          <InsetRow label="Expires">
            <span className="stopped">Not yet. Never checked.</span>
          </InsetRow>
        ) : state === 'lapsed' ? (
          <InsetRow label="Expired">
            <b className="danger-title">{expires(expiry)}</b>
            <small>The agent renews it. Nothing to do here.</small>
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
              <small>local alias {account.alias}</small>
            </>
          ) : (
            <>
              No account on this server<small>You can still read from it</small>
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
            'None yet'
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
              <small>Hover for the full ID. Copy copies all of it.</small>
            </InsetRow>
            <InsetRow label="Signed history">
              {host.chain} entries · version {host.epoch}
            </InsetRow>
          </Inset>
          <Toggle
            label="Inspect last check response"
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
                ? 'Set by the first check'
                : state === 'pending'
                  ? 'Reading signed status…'
                  : 'Hidden while locked'}
            </span>
          </InsetRow>
        </Inset>
      )}

      <SectionLabel className="danger-title">On this Mac</SectionLabel>
      <Inset className="settings-inset middle danger-box">
        <InsetRow
          className="dangerrow"
          label="Remove local server data"
          action={
            <Button
              size="sm"
              icon="trash"
              disabled={blocked}
              onClick={onForget}
            >
              Remove local data…
            </Button>
          }
        >
          <small>Removes it from this Mac.</small>
        </InsetRow>
        <InsetRow
          className="dangerrow"
          label="Reset local state"
          action={
            <Button size="sm" variant="danger" onClick={onReset}>
              Reset…
            </Button>
          }
        >
          <small>
            Drops the pinned identity, cache and unfinished operations.
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
      subtitle="Save its address, then check its identity"
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
        Adding a server saves its profile and address on this Mac. Check it next
        to verify and pin its signed host identity.
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
  bridge,
  onClose,
  onReset,
  onError,
}: {
  server: Server;
  preview: ResetPreview | null;
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
      title={`Reset ${server.name}?`}
      subtitle="Discard only this Mac’s local state for the whole server"
      onClose={onClose}
      danger
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
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
            Reset local state
          </Button>
        </>
      }
    >
      <p>
        This does not delete server data. Check again before using this server.
      </p>
      <Inset>
        <InsetRow label="Discarded">
          Pinned Host ID, signed-history checkpoint and cached server artifacts.
        </InsetRow>
        <InsetRow label="Lost">
          Local writes that were never accepted by the server cannot be
          recovered.
        </InsetRow>
        <InsetRow label="Kept">
          Your keys and account remain on this Mac, but this reset does not sign
          you in.
        </InsetRow>
        <InsetRow label="Untouched">
          Every other configured server and its local state.
        </InsetRow>
      </Inset>
      {preview ? (
        <>
          <SectionLabel>Also discarded · resumable operations</SectionLabel>
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
              <InsetRow label="None">
                No resumable operation was reported.
              </InsetRow>
            )}
          </Inset>
          <SectionLabel>Local artifacts discarded</SectionLabel>
          <Inset>
            {preview.artifacts.length ? (
              preview.artifacts.map((row) => (
                <InsetRow key={row.kind} label={row.kind}>
                  {row.entries} entries · {row.bytes.toLocaleString()} bytes
                </InsetRow>
              ))
            ) : (
              <InsetRow label="None">
                No local artifacts were reported.
              </InsetRow>
            )}
          </Inset>
          <p className="hint">
            This preview token expires in {preview.expiresInSeconds} seconds and
            can be used once.
            {available ? '' : ' Preview again by closing and reopening Reset.'}
          </p>
        </>
      ) : (
        <p>Reading the exact reset preview…</p>
      )}
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${server.id}`}
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
      title={`Remove local data for ${server.name}?`}
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
            Remove local server data
          </Button>
        </>
      }
    >
      <p>
        The server and its ciphertext are unchanged. Type the local profile name
        to confirm.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${server.id}`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}
