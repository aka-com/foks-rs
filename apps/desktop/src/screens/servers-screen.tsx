/** Device-wide server inventory and server details embedded in Settings › Account. */

import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import type {
  Bridge,
  CheckedServer,
  ServerStatusSnapshot,
  StoredHost,
} from '../bridge';
import { useServerMetadata } from './servers/use-server-metadata';
import { canReadServer } from './servers/server-workflow';
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
import { PageHeader } from '../shell/page-header';
import { AccountMark } from './account-switcher';
import { ProfileKeys } from './profile-keys';
import { GroupMark } from './group-mark';

interface Props {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The server the section is open on, or nothing for the list. */
  profile?: string;
  /** The Account page to retain while opening and closing server details. */
  store?: StoreRef;
  /** The named fixture scene Settings was entered at, captured once there. */
  scene: string;
  /** Account tab panel identity, present when one server owns the page. */
  panel?: { id: string; labelledBy: string };
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}

type Sheet = 'add' | 'rename' | 'remove' | null;

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

/** The identity pinned for this server, as the catalog carries it. */
type PinnedHost = Pick<StoredHost, 'hostId' | 'chain' | 'epoch'>;

function pinnedHost(server: Server): PinnedHost | null {
  // A server whose trust is blocked, or whose saved schema this client
  // cannot read, states no identity here; the page says so instead. The
  // three fields are projected from one signed status, so either all of them
  // describe a pinned host or none of them does.
  return canReadServer(server) &&
    server.host_id !== null &&
    server.chain !== null &&
    server.epoch !== null
    ? { hostId: server.host_id, chain: server.chain, epoch: server.epoch }
    : null;
}

/**
 * When this server's signed compatibility result expires, in Unix seconds,
 * or null where the pinned protocol states no expiry. The expiry belongs to
 * the signed result itself, which the catalog carries, so it is read from
 * there; a check's own status read is newer than the catalog refresh it asks
 * for and answers until that refresh lands.
 */
function leaseExpiry(
  server: Server,
  checked?: ServerStatusSnapshot,
): number | null {
  const lease = checked?.compatibility ?? server.compatibility;
  return 'expiresAt' in lease ? lease.expiresAt : null;
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
  // A server that answered is the ordinary case, and the row's mark already
  // reads as one: only a state worth acting on carries a chip.
  if (state === 'unprobed' || state === 'checked') return null;
  return (
    <Chip tone={state === 'pending' ? 'default' : 'bad'}>
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

/** Returns the Account location for the server list or one server detail. */
const servers = (profile?: string, store?: StoreRef): Location => ({
  kind: 'settings',
  section: 'account',
  ...(store ? { store } : {}),
  ...(profile ? { profile } : {}),
});

export function ServersSection({
  snapshot: agentSnapshot,
  bridge,
  profile,
  store,
  scene,
  panel,
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
        ? 'remove'
        : null,
  );
  const toasts = useToast();
  const selected = serverFor(agentSnapshot, profile);
  const { statuses, checked, busy, check } = useServerMetadata({
    bridge,
    snapshot: agentSnapshot,
    profile,
    enteredScene,
    onMutationError,
    onRefresh,
    toast: (message) => toasts.show(message),
  });

  // A check pins or advances the server's identity and then reads its signed
  // status back; the section is where both answers are stated. Nothing else
  // here is typed, so this is the only thing the section answers for.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the server check to finish.' }
      : null,
  );

  const currentHost = selected
    ? (checked.get(selected.id) ?? pinnedHost(selected))
    : null;
  const selectedState = selected
    ? resolveServerUiState(agentSnapshot, selected)
    : null;
  const selectedAccount = selected
    ? agentSnapshot.accounts.find((item) => item.server === selected.id)
    : undefined;

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
          await onRefresh('Server verified and added.');
          onNavigate(servers(added, store));
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
    ) : sheet === 'remove' && selected ? (
      <RemoveServerSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onRemoved={async () => {
          setSheet(null);
          onNavigate(servers(undefined, store));
          await onRefresh(
            `Removed ${serverLocalAlias(selected)} and its credentials`,
          );
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : null;

  const content = selected ? (
    <>
      <ServerBody
        snapshot={agentSnapshot}
        server={selected}
        status={statuses.get(selected.id)}
        host={currentHost}
        checked={checked.get(selected.id)}
        busy={busy}
        onCheck={() => void check(selected)}
        onRemove={() => setSheet('remove')}
        onCopy={(text) =>
          void bridge
            .copyText(text)
            .then(() => toasts.show('Host ID copied to clipboard'))
            .catch(onError)
        }
        onOpenGroup={(store) => onNavigate({ kind: 'store', ref: store.id })}
        onOpenAccount={(store) =>
          onNavigate({ kind: 'settings', section: 'account', store })
        }
      />
      <ProfileKeys
        key={selected.id}
        snapshot={agentSnapshot}
        server={selected}
        bridge={bridge}
        onError={onError}
      />
    </>
  ) : (
    <ServerList
      snapshot={agentSnapshot}
      statuses={statuses}
      busy={busy}
      onOpen={(next) => onNavigate(servers(next, store))}
      onCheck={(server) => void check(server)}
      onAdd={() => setSheet('add')}
    />
  );

  if (selected && selectedState)
    return (
      <>
        <PageHeader
          ruled
          title={serverLocalAlias(selected)}
          mark={<ServerMark state={selectedState} />}
          sub={
            selectedAccount && !isLocked(selectedState)
              ? `Signed in as ${selectedAccount.username}`
              : selected.configuredProbe
          }
          action={
            <>
              <StatusChip state={selectedState} />
              <Button size="sm" onClick={() => setSheet('rename')}>
                Rename…
              </Button>
              <Button
                size="sm"
                icon="again"
                disabled={
                  busy ||
                  selectedState === 'blocked' ||
                  selectedState === 'recovery-required'
                }
                title={
                  selectedState === 'recovery-required'
                    ? 'Restore saved security state before checking this identity.'
                    : selectedState === 'blocked'
                      ? 'This server is blocked because its identity changed'
                      : 'Verify the saved server identity; this does not renew compatibility permission.'
                }
                onClick={() => void check(selected)}
              >
                Check
              </Button>
            </>
          }
        />
        <div
          className="body"
          role="tabpanel"
          id={panel?.id}
          aria-labelledby={panel?.labelledBy}
        >
          <div className="settings-main">{content}</div>
        </div>
        {overlay}
      </>
    );

  return (
    <>
      {content}
      {overlay}
    </>
  );
}

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
        {state === 'checked' ? null : (
          <small>
            <StatusLine state={state} expiry={expiry} />
          </small>
        )}
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
    expiry: leaseExpiry(server, statuses.get(server.id)),
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
            key={server.id}
            server={server}
            state={state}
            expiry={expiry}
            busy={busy}
            onOpen={onOpen}
            onCheck={onCheck}
          />
        ))}
        {!rows.length ? (
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

/**
 * Displays an alert banner and primary action for servers requiring attention.
 */
function StatusBand({
  state,
  expiry,
  busy,
  onCheck,
  onRemove,
}: {
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onCheck: () => void;
  onRemove: () => void;
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
          <Button size="sm" variant="danger" onClick={onRemove}>
            Remove…
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
  busy,
  onCheck,
  onRemove,
  onCopy,
  onOpenGroup,
  onOpenAccount,
}: {
  snapshot: AgentSnapshot;
  server: Server;
  /** The signed status this page checked itself; see `leaseExpiry`. */
  status?: ServerStatusSnapshot;
  host: PinnedHost | null;
  checked?: CheckedServer;
  busy: boolean;
  onCheck: () => void;
  onRemove: () => void;
  onCopy: (text: string) => void;
  onOpenGroup: (store: TeamStore) => void;
  /** The account on this server, on Account, where an account is managed. */
  onOpenAccount: (store: StoreRef) => void;
}): ReactNode {
  const state = resolveServerUiState(agentSnapshot, server);
  const locked = isLocked(state);
  const blocked = state === 'blocked';
  const expiry = leaseExpiry(server, status);
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
  const hasHost = Boolean(host) && state !== 'unavailable';

  return (
    <>
      <StatusBand
        state={state}
        expiry={expiry}
        busy={busy}
        onCheck={onCheck}
        onRemove={onRemove}
      />
      {checked?.serverVersion && !checked.serverVersion.compatible ? (
        <Band severity="warn" label="Version mismatch">
          {versionMismatchText(checked.serverVersion)}
        </Band>
      ) : null}

      <Inset className="settings-inset middle">
        {state === 'checked' ? (
          <>
            {/* The header carries the one Check this page offers; a checked
                server's status row states the fact and nothing else. */}
            <InsetRow label="Status">
              {checked
                ? 'Last checked now. No issues.'
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
            Verified · {plural(host.chain, 'entry', 'entries')} · Checkpoint{' '}
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
          label="Remove server and credentials"
          action={
            <Button size="sm" icon="trash" variant="danger" onClick={onRemove}>
              Remove…
            </Button>
          }
        >
          <small>
            Removes this server, its credentials, trust history, cached data,
            and unfinished operations from this device.
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
  // Suggestions, never values: an address that was not typed must not be
  // submittable, and a profile name is the reader's to choose.
  const [profile, setProfile] = useState('');
  const [probe, setProbe] = useState('');
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
                .checkAndAddProfile(profile, cleanProbe)
                .then((added) => onAdded(added.profile))
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
        Add FOKS protocol v019 servers here. The server is saved only after
        online verification succeeds.
      </p>
      <Inset>
        <Field
          label="Server name"
          value={profile}
          placeholder="new-foks"
          onChange={setProfile}
        />
        <Field
          label="Address"
          value={probe}
          placeholder="foks.example.com"
          onChange={setProbe}
        />
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

function RemoveServerSheet({
  server,
  bridge,
  onClose,
  onRemoved,
  onError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onRemoved: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  // Removing a server deletes what this device holds for it.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetFrame
      title={`Remove ${serverLocalAlias(server)} and its credentials?`}
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
                .removeServerAndCredentials(server.id, confirmation)
                .then((removed) => {
                  if (removed.profile !== server.id)
                    throw new Error(
                      'remove_server_and_credentials returned a different profile.',
                    );
                  return onRemoved();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Remove server and credentials
          </Button>
        </>
      }
    >
      <p>
        Your data on the server is not deleted. Type the server name to confirm.
      </p>
      <Inset>
        <InsetRow label="Removed">
          This server, all of its credentials, pinned host identity, connection
          history, cached data, and unfinished operations.
        </InsetRow>
        <InsetRow label="Warning">
          Accounts without a paper key or another paired device cannot be
          accessed again.
        </InsetRow>
        <InsetRow label="Unaffected">
          Other configured servers and their local data.
        </InsetRow>
      </Inset>
      <Inset className="confirm-inset">
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
