/**
 * The Servers section of Settings, displaying configured servers and detailed server state.
 */

import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import type { Bridge, CheckedServer, ServerStatusSnapshot } from '../bridge';
import { useServerMetadata } from './servers/use-server-metadata';
import { useResetWorkflow } from './servers/use-reset-workflow';
import { serverBinding } from './servers/server-workflow';
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
import { useSheetGuard } from '../navigation-guard';
import {
  plural,
  serverAvailability,
  serverLocalAlias,
  shortId,
  storeAttentionState,
  storeDescription,
  teamCaption,
} from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import type { Server, StoreRef, TeamStore, AgentSnapshot } from '../model';
import { AccountMark } from './account-switcher';
import { ProfileKeys } from './profile-keys';
import { GroupMark } from './group-mark';

interface Props {
  snapshot: AgentSnapshot;
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

type Sheet = 'add' | 'rename' | 'reset' | 'forget' | null;

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
/** Explains why a checked server does not accept this client's v0.1.9. */
function versionMismatchText(
  version: NonNullable<CheckedServer['serverVersion']>,
): string {
  const bounds: string[] = [];
  if (version.minimum) bounds.push(`requires ${version.minimum} or newer`);
  if (version.newest) bounds.push(`supports up to ${version.newest}`);
  const range = bounds.length ? `; it ${bounds.join(' and ')}` : '';
  const message = version.message ? ` ${version.message}` : '';
  return `This server does not accept FOKS client 0.1.9${range}.${message}`;
}

function serverFor(
  agentSnapshot: AgentSnapshot,
  profile: string | undefined,
): Server | undefined {
  return profile
    ? agentSnapshot.servers.find((server) => server.id === profile)
    : undefined;
}

/** UI state representing server health and connectivity. */
type ServerUiState =
  | 'checked'
  | 'unprobed'
  | 'lapsed'
  | 'unavailable'
  | 'blocked'
  | 'schema'
  | 'incompatible'
  | 'import-verification'
  | 'recovery-required'
  | 'pending';

const STATE_LABEL: Readonly<Record<ServerUiState, string>> = {
  checked: 'Checked',
  unprobed: 'Never checked',
  lapsed: 'Check-in expired',
  unavailable: 'Status unknown',
  blocked: 'Untrusted',
  schema: 'Schema incompatible',
  incompatible: 'Protocol incompatible',
  'import-verification': 'Verification required',
  'recovery-required': 'Security state missing',
  pending: 'Checking status',
};

/** Returns true if the server state blocks read and write operations. */
const isLocked = (state: ServerUiState): boolean =>
  state === 'lapsed' ||
  state === 'unavailable' ||
  state === 'blocked' ||
  state === 'schema' ||
  state === 'incompatible' ||
  state === 'import-verification' ||
  state === 'recovery-required';

const markTone = (state: ServerUiState): string =>
  state === 'checked' ? 'ok' : isLocked(state) ? 'bad' : '';

function resolveServerUiState(
  agentSnapshot: AgentSnapshot,
  server: Server,
): ServerUiState {
  const availability = serverAvailability(agentSnapshot, server);
  if (availability.available) return 'checked';
  if (availability.reason === 'loading') return 'pending';
  if (availability.reason === 'compatibility-incompatible')
    return 'incompatible';
  if (availability.reason === 'security-state-missing')
    return 'recovery-required';
  if (availability.reason === 'verification-required') return 'unprobed';
  if (availability.reason === 'check-in-expired') return 'lapsed';
  if (availability.reason === 'verification-failed') return 'blocked';
  if (availability.reason === 'schema-incompatible') return 'schema';
  if (availability.reason === 'import-verification-required')
    return 'import-verification';
  return 'unavailable';
}

function StatusChip({ state }: { state: ServerUiState }): ReactNode {
  if (state === 'unprobed') return null;
  return (
    <Chip
      tone={
        state === 'checked' ? 'ok' : state === 'pending' ? 'default' : 'bad'
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
  snapshot: agentSnapshot,
  bridge,
  profile,
  scene,
  onNavigate,
  onRefresh,
  onError,
  onMutationError,
}: Props): ReactNode {
  const [enteredScene] = useState(scene);
  const [sheet, setSheet] = useState<Sheet>(() =>
    enteredScene === 'servers-add'
      ? 'add'
      : enteredScene === 'servers-reset'
        ? 'reset'
        : null,
  );
  const toasts = useToast();
  const selected = serverFor(agentSnapshot, profile);
  const rollback = enteredScene === 'servers-rollback';
  const { statuses, checked, busy, check } = useServerMetadata({
    bridge,
    snapshot: agentSnapshot,
    profile,
    enteredScene,
    onError,
    onMutationError,
    onRefresh,
    toast: (message) => toasts.show(message),
  });

  useEffect(() => {
    const conceal = (): void => {
      setSheet(null);
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

  // A check pins or advances the server's identity and then reads its signed
  // status back; the section is where both answers are stated. Nothing else
  // here is typed, so this is the only thing the section answers for.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the server check to finish.' }
      : null,
  );

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
        key={serverBinding(selected)}
        server={selected}
        bridge={bridge}
        onPreviewError={onError}
        onClose={() => setSheet(null)}
        onReset={async () => {
          setSheet(null);
          await onRefresh(
            `${serverLocalAlias(selected)} has been reset. Verify the server before reconnecting.`,
          );
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : sheet === 'rename' && selected ? (
      <RenameServerSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onRenamed={async () => {
          await onRefresh('Server name updated.');
          setSheet(null);
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
          await onRefresh(`Removed ${serverLocalAlias(selected)}`);
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : null;

  return (
    <>
      {selected ? (
        <ServerBody
          snapshot={agentSnapshot}
          server={selected}
          status={statuses.get(selected.id)}
          host={currentHost}
          checked={checked.get(selected.id)}
          rollback={rollback}
          busy={busy}
          onBack={() => onNavigate(servers())}
          onCheck={() => void check(selected)}
          onRename={() => setSheet('rename')}
          onReset={() => setSheet('reset')}
          onForget={() => setSheet('forget')}
          onCopy={(text) =>
            void bridge
              .copyText(text)
              .then(() => toasts.show('Host ID copied to clipboard'))
              .catch(onError)
          }
          onOpenGroup={(store) => onNavigate({ kind: 'store', ref: store.id })}
          onOpenAccount={(store) => onNavigate({ kind: 'people', store })}
        />
      ) : (
        <ServerList
          snapshot={agentSnapshot}
          statuses={statuses}
          busy={busy}
          onOpen={(next) => onNavigate(servers(next))}
          onCheck={(server) => void check(server)}
          onAdd={() => setSheet('add')}
        />
      )}
      {selected ? (
        <ProfileKeys
          key={selected.id}
          snapshot={agentSnapshot}
          server={selected}
          bridge={bridge}
          onError={onError}
        />
      ) : null}
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
        {groups.length ? plural(groups.length, 'team') : 'No teams'}
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
  snapshot: agentSnapshot,
  server,
  state,
  expiry,
  busy,
  onOpen,
  onCheck,
}: {
  snapshot: AgentSnapshot;
  server: Server;
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
}): ReactNode {
  const account = agentSnapshot.accounts.find(
    (item) => item.server === server.id,
  );
  const groups = agentSnapshot.stores
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
          {/* A never-checked server and a lapsed one are equally stuck, and
              the command is the same one, so both rows offer it. */}
          {state === 'unprobed' || state === 'lapsed' ? (
            <Button
              size="sm"
              variant={state === 'unprobed' ? 'primary' : 'plain'}
              icon="again"
              disabled={busy}
              onClick={() => onCheck(server)}
            >
              Check
            </Button>
          ) : null}
          <Button size="sm" onClick={() => onOpen(server.id)}>
            Manage
          </Button>
        </>
      }
    >
      <ServerMark state={state} />
      <span className="t">
        <b>
          <span>{serverLocalAlias(server)}</span>
        </b>
        {server.configuredProbe !== serverLocalAlias(server) && (
          <small>{server.configuredProbe}</small>
        )}
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
  snapshot: agentSnapshot,
  statuses,
  busy,
  onOpen,
  onCheck,
  onAdd,
}: {
  snapshot: AgentSnapshot;
  statuses: Map<string, ServerStatusSnapshot>;
  busy: boolean;
  onOpen: (profile: string) => void;
  onCheck: (server: Server) => void;
  onAdd: () => void;
}): ReactNode {
  const rows = agentSnapshot.servers.map((server) => {
    const snapshot = statuses.get(server.id);
    const state = resolveServerUiState(agentSnapshot, server);
    const expiry =
      server.compatibility.status === 'required'
        ? server.compatibility.expiresAt
        : (snapshot?.leaseExpiresAt ?? null);
    return { server, state, expiry };
  });
  // Servers requiring user attention are displayed at the top of the list. A
  // server that has never been checked is one of them: nothing on it can be
  // used until it is, so Ready would be a false claim about it.
  const needsAttention = (row: (typeof rows)[number]): boolean =>
    isLocked(row.state) || row.state === 'unprobed';
  const attention = rows.filter(needsAttention);
  const ready = rows.filter((row) => !needsAttention(row));
  const box = (entries: typeof rows): ReactNode => (
    <Inset className="settings-inset middle">
      {entries.map(({ server, state, expiry }) => (
        <ServerRow
          key={server.id}
          snapshot={agentSnapshot}
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
          <SectionLabel action={add}>Servers on this device</SectionLabel>
          <Inset className="settings-inset">
            <div className="sempty">
              <ServerMark state="unprobed" />
              <b>No servers on this device yet</b>
              <p>Add a server using the field above to begin.</p>
            </div>
          </Inset>
        </>
      ) : null}
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
        label="Not checked yet"
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
        label="Check-in expired"
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
        Signed compatibility verification expired {expires(expiry)}. A newer
        signed result is required; checking the server identity does not renew
        it.
      </Band>
    );
  if (state === 'recovery-required')
    return (
      <Band severity="crit" label="Saved security state is missing">
        Restore or inspect the saved client security state before reconnecting.
        FOKS will not establish a replacement identity automatically.
      </Band>
    );
  if (state === 'unavailable')
    return (
      <Band severity="crit" label="Check-in status unknown">
        Cannot verify the status of this server. This server is locked until a
        valid status is confirmed. Use the Check button in the header to check
        status.
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
        identity on this device. Access has been blocked for your security.
      </Band>
    );
  if (state === 'incompatible')
    return (
      <Band severity="crit" label="Protocol incompatible">
        Compatibility verification does not permit operations with this server.
        This is not an identity mismatch; resetting trust will not resolve it.
      </Band>
    );
  if (state === 'schema')
    return (
      <Band severity="crit" label="Server schema incompatible">
        This client cannot safely read the server’s saved schema. Review the
        supported versions; FOKS will not reset this data automatically.
      </Band>
    );
  if (state === 'import-verification')
    return (
      <Band severity="crit" label="Imported profile needs verification">
        Verify this imported profile online before accessing its vaults. Use the
        Check button in the header to verify the profile.
      </Band>
    );
  return null;
}

function ServerBody({
  snapshot: agentSnapshot,
  server,
  status,
  host,
  checked,
  rollback,
  busy,
  onBack,
  onCheck,
  onRename,
  onReset,
  onForget,
  onCopy,
  onOpenGroup,
  onOpenAccount,
}: {
  snapshot: AgentSnapshot;
  server: Server;
  status?: ServerStatusSnapshot;
  host: ServerStatusSnapshot['host'];
  checked?: CheckedServer;
  rollback: boolean;
  busy: boolean;
  onBack: () => void;
  onCheck: () => void;
  onRename: () => void;
  onReset: () => void;
  onForget: () => void;
  onCopy: (text: string) => void;
  onOpenGroup: (store: TeamStore) => void;
  /** The account on this server, on Account, where an account is managed. */
  onOpenAccount: (store: StoreRef) => void;
}): ReactNode {
  const state = resolveServerUiState(agentSnapshot, server);
  const locked = isLocked(state);
  const blocked = state === 'blocked';
  const expiry =
    server.compatibility.status === 'required'
      ? server.compatibility.expiresAt
      : (status?.leaseExpiresAt ?? null);
  const account = agentSnapshot.accounts.find(
    (item) => item.server === server.id,
  );
  // Teams that need attention — setup incomplete, and the like — list after
  // the ones in a normal state, so the working teams read first.
  const groups = agentSnapshot.stores
    .filter(
      (store): store is TeamStore =>
        store.kind === 'team' && store.server === server.id,
    )
    .sort(
      (a, b) =>
        Number(storeAttentionState(agentSnapshot, a) !== 'normal') -
        Number(storeAttentionState(agentSnapshot, b) !== 'normal'),
    );
  const subtitle =
    account && !locked ? `Signed in as ${account.username}` : null;
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
          <b>{serverLocalAlias(server)}</b>
          {subtitle ? <small>{subtitle}</small> : null}
        </span>
        {/* The page's own mark already reads as a healthy server; only an
            abnormal state is worth a chip beside it. The list row keeps its
            Checked chip, where the mark is smaller. */}
        {state === 'checked' ? null : <StatusChip state={state} />}
        <Button size="sm" onClick={onRename}>
          Rename…
        </Button>
        <Button
          size="sm"
          icon="again"
          disabled={busy || blocked || state === 'recovery-required'}
          title={
            state === 'recovery-required'
              ? 'Restore saved security state before checking this identity.'
              : blocked
                ? 'This server is blocked until its identity is reset'
                : 'Verify the saved server identity; this does not renew compatibility permission.'
          }
          onClick={onCheck}
        >
          Check
        </Button>
      </div>
      <StatusBand
        state={state}
        expiry={expiry}
        busy={busy}
        onCheck={onCheck}
        onReset={onReset}
      />
      {checked?.serverVersion && !checked.serverVersion.compatible ? (
        <Band severity="warn" label="Version mismatch">
          {versionMismatchText(checked.serverVersion)}
        </Band>
      ) : null}

      <SectionLabel>Check-in</SectionLabel>
      <Inset className="settings-inset middle">
        {state === 'checked' ? (
          <>
            {/* The header carries the one Check this page offers; a checked
                server's status row states the fact and nothing else. */}
            <InsetRow label="Status">
              {checked
                ? 'Last checked: now. Trust history unchanged.'
                : 'Identity pinned on this device.'}
            </InsetRow>
            <InsetRow label="Expires">{expires(expiry)}</InsetRow>
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

      {/* The whole response, behind one disclosure, directly under the
          check-in it is the record of. */}
      {hasHost && host ? (
        <Toggle
          label="Inspect last check response"
          open={rollback || undefined}
          disabled={rollback}
        >
          <pre>
            {JSON.stringify(
              {
                profile: status?.profile ?? server.id,
                configuredProbe: status?.configuredProbe ?? null,
                host,
                leaseRequired: status?.leaseRequired ?? null,
                leaseExpiresAt: expiry,
              },
              null,
              2,
            )}
          </pre>
        </Toggle>
      ) : null}

      {/* What stops if this server lapses: the account held on it, and the
          groups that live there. */}
      <SectionLabel>Accounts on this server</SectionLabel>
      <Inset className="settings-inset middle wide">
        {locked ? (
          <InsetRow label="You">
            <span className="stopped">Hidden while locked</span>
          </InsetRow>
        ) : account ? (
          <InsetRow
            className="devrow"
            action={
              <Button size="sm" onClick={() => onOpenAccount(account.store)}>
                Account
              </Button>
            }
          >
            <AccountMark name={account.username} />
            <span className="t">
              <span className="namechip">
                <b>{account.username}</b> <Chip>{account.alias}</Chip>
              </span>
            </span>
          </InsetRow>
        ) : (
          <InsetRow label="You">
            No account on this server<small>Read-only access available</small>
          </InsetRow>
        )}
      </Inset>

      <SectionLabel>Teams on this server</SectionLabel>
      <Inset className="settings-inset middle wide">
        {locked ? (
          <InsetRow label="Teams">
            <span className="stopped">
              {groups.length
                ? `${groups.map((store) => store.name).join(', ')} — locked`
                : 'Hidden while locked'}
            </span>
          </InsetRow>
        ) : groups.length ? (
          groups.map((store) => {
            const description = storeDescription(agentSnapshot, store);
            const abnormal =
              storeAttentionState(agentSnapshot, store) !== 'normal';
            return (
              <InsetRow
                key={store.id}
                className="devrow"
                action={
                  <>
                    {abnormal ? <Chip tone="warn">{description}</Chip> : null}
                    <Button
                      size="sm"
                      aria-label={`Open ${store.name}`}
                      onClick={() => onOpenGroup(store)}
                    >
                      Open
                    </Button>
                  </>
                }
              >
                <GroupMark store={store} size="sm" />
                <span className="t">
                  <b>{store.name}</b>
                  <small>
                    {/* This page is already about one server, so the caption
                        does not repeat it. */}
                    {teamCaption(agentSnapshot, store, { server: false })}
                    {abnormal ? '' : ` · ${description}`}
                  </small>
                </span>
              </InsetRow>
            );
          })
        ) : (
          <InsetRow label="Teams">No teams on this server</InsetRow>
        )}
      </Inset>

      <SectionLabel>Identity and trust</SectionLabel>
      {hasHost && host ? (
        <Inset className="settings-inset middle">
          <InsetRow label="Internal ID">{server.id}</InsetRow>
          <InsetRow label="Address">
            {status?.configuredProbe ?? server.configuredProbe}
          </InsetRow>
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
          </InsetRow>
          {/* `chain` and `epoch` are a length and a checkpoint number, not
              times: they are reported as the two numbers the agent sends. */}
          <InsetRow label="Audit log">
            Verified · {plural(host.chain, 'entry', 'entries')} · checkpoint{' '}
            {host.epoch}
          </InsetRow>
        </Inset>
      ) : (
        <Inset className="settings-inset middle">
          <InsetRow label="Internal ID">{server.id}</InsetRow>
          <InsetRow label="Address">
            {status?.configuredProbe ?? server.configuredProbe}
          </InsetRow>
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

      <SectionLabel className="danger-title">Danger zone</SectionLabel>
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
          <small>Removes this server from this device.</small>
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
  children,
  footer,
  onClose,
  danger = false,
}: {
  title: string;
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
      <p>The server is verified after it is added.</p>
      <Inset>
        <Field label="Profile" value={profile} onChange={setProfile} />
        <Field label="Address" value={probe} onChange={setProbe} />
      </Inset>
    </SheetFrame>
  );
}

function RenameServerSheet({
  server,
  bridge,
  onClose,
  onRenamed,
  onError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onRenamed: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [name, setName] = useState(server.label ?? server.name);
  const [busy, setBusy] = useState(false);
  const trimmed = name.trim();
  const valid =
    !trimmed.includes('\0') &&
    !trimmed.includes('\n') &&
    !trimmed.includes('\r') &&
    new TextEncoder().encode(trimmed).length <= 64;
  return (
    <SheetFrame
      title="Rename server"
      onClose={onClose}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              const label =
                trimmed === '' || trimmed === server.name ? null : trimmed;
              setBusy(true);
              void bridge
                .setServerLabel(server.id, label)
                .then((response) => {
                  if (
                    response.profile !== server.id ||
                    response.label !== label
                  )
                    throw new Error(
                      'set_server_label returned a different server label.',
                    );
                  return onRenamed();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Save
          </Button>
        </>
      }
    >
      <Inset>
        <Field label="Display name" value={name} onChange={setName} />
      </Inset>
    </SheetFrame>
  );
}

function ResetSheet({
  server,
  bridge,
  onClose,
  onReset,
  onError,
  onPreviewError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onReset: () => Promise<void>;
  onError: (error: unknown) => void;
  onPreviewError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const {
    preview,
    loading: resetLoading,
    error: resetError,
    available,
    busy,
    load: onRetryPreview,
    close,
    reset,
  } = useResetWorkflow({ bridge, server, onClose, onError, onPreviewError });
  // The reset spends its one token and deletes local keys; the sheet is where
  // it says whether it did.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the reset to finish.' }
      : null,
  );
  return (
    <SheetFrame
      title={`Erase local credentials for ${serverLocalAlias(server)}?`}
      onClose={close}
      danger
      footer={
        <>
          <Button disabled={busy} onClick={close}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={
              confirmation !== server.id || !preview || !available || busy
            }
            onClick={() => void reset(confirmation, onReset)}
          >
            Erase credentials and reset
          </Button>
        </>
      }
    >
      <p>
        Deletes this device's account keys for this server. Your data stays on
        the server, but without another device or a paper key you cannot get
        back into the account. The passphrase alone is not enough.
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
          Verify the server again and recover or pair an account before using it
          on this device. Saving a paper key here is not part of this operation.
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
            This confirmation expires in {preview.expiresInSeconds} seconds.
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
          <Button disabled={resetLoading} onClick={() => void onRetryPreview()}>
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
  // Forgetting a server deletes what this Mac holds for it.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetFrame
      title={`Remove local data for ${serverLocalAlias(server)}?`}
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
        Your data on the server is not deleted. Type the server name to confirm.
      </p>
      <Inset>
        <InsetRow label="Removed">
          All credentials for this server, pinned host identity, connection
          history, and cached data.
        </InsetRow>
        <InsetRow label="Warning">
          Accounts without a paper key or another paired device cannot be
          accessed again.
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
