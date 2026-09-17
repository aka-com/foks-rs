import { InvitationPanel } from '../components/invitation-panel';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  Field,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  MenuButton,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
  Tabs,
  Toggle,
} from '../components';
import {
  actionableGroupMember,
  admissionActive,
  catalog,
  canCreateInStore,
  formatRole,
  groupDetailFailure,
  hue,
  kindOf,
  parseRole,
  partiesOf,
  partyName,
  peopleGroups,
  plural,
  readersOf,
  roleRank,
  serverOf,
  storeDescriptionState,
  storeOf,
  storeReadable,
  visibilityOf,
} from '../model';
import type {
  Account,
  AccountStore,
  FederationEntry,
  GroupDetailFailure,
  Item,
  Party,
  Server,
  Store,
  StoreRef,
  World,
} from '../model';
import { enqueueProfileWork } from '../bridge';
import type { Bridge, PendingOperation } from '../bridge';
import type { RoleDto } from '../bridge';
import type { GroupSettingsTab, Location } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { StoreAccessTakeover } from './store-access';
import { useToast } from '/kit/toasts';

type Tab = GroupSettingsTab;
export type GroupSheetKind =
  'invite' | 'add' | 'demote' | 'remove' | 'admit' | 'create';
type Sheet = GroupSheetKind | null;

const VIS_MIN = -32768;
const VIS_MAX = 32767;
const stateName = (): string =>
  typeof window === 'undefined'
    ? ''
    : (new URLSearchParams(window.location.search).get('state') ?? '');

function itemsOf(world: World, store: string): Item[] {
  return catalog(world).filter((item) => item.store === store);
}

function readsOf(world: World, party: Party): Item[] {
  return itemsOf(world, party.store).filter((item) =>
    readersOf(world, item)?.some(
      (candidate) => candidate.party_id_hex === party.party_id_hex,
    ),
  );
}

function canTarget(world: World, party: Party): boolean {
  if (!actionableGroupMember(world, party)) return false;
  const parties = partiesOf(world, party.store);
  const mine = parties.find((candidate) => candidate.label === 'you');
  if (!mine) return false;
  const myRank = roleRank(mine.destination_role);
  const targetRank = roleRank(party.destination_role);
  if (myRank < 2) return false;
  if (targetRank >= 3 && myRank < 3) return false;
  return true;
}

function demotionFor(party: Party): RoleDto | null {
  const role = parseRole(party.destination_role);
  if (!role) return null;
  if (role.kind === 'owner' || role.kind === 'admin')
    return { role: 'Member', visibility: 0 };
  const visibility = role.visibility ?? 0;
  return visibility > VIS_MIN
    ? { role: 'Member', visibility: visibility - 1 }
    : null;
}

function roleText(party: Party): string {
  return fmtRole(party.destination_role);
}

function partyShortName(party: Party): string {
  return party.party_kind === 'user'
    ? partyName(party)
    : (party.team_name?.split(' @')[0] ?? partyName(party));
}

function fmtRole(role: Item['read']): string {
  const parsed = parseRole(role);
  return parsed
    ? formatRole(parsed)
    : typeof role === 'string'
      ? role
      : role.role;
}

function roleName(role: Item['read']): string {
  const parsed = parseRole(role);
  if (!parsed) return typeof role === 'string' ? role : role.role;
  return parsed.kind === 'owner'
    ? 'Owner'
    : parsed.kind === 'admin'
      ? 'Admin'
      : 'Member';
}

function RoleChip({
  role,
  extra,
}: {
  role: Item['read'] | RoleDto | null | undefined;
  extra?: ReactNode;
}): ReactNode {
  if (!role) {
    return (
      <span className="rolecell">
        <Chip>—</Chip>
      </span>
    );
  }
  const parsed = parseRole(role);
  return (
    <span className="rolecell">
      <Chip>{roleName(role)}</Chip>
      {/* Access levels apply only to Member roles; Owner and Admin roles have full visibility. */}
      {parsed?.kind === 'member' ? (
        <small>visibility {visibilityOf(parsed)}</small>
      ) : null}
      {extra ? <small>{extra}</small> : null}
    </span>
  );
}

function oxfordOr(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  if (names.length === 2) return `${names[0]} or ${names[1]}`;
  return `${names.slice(0, -1).join(', ')}, or ${names[names.length - 1]}`;
}

function sortRoster(parties: readonly Party[]): Party[] {
  return [...parties].sort((left, right) => {
    const rank =
      roleRank(right.destination_role) - roleRank(left.destination_role);
    if (rank) return rank;
    if (left.label === 'you' && right.label !== 'you') return -1;
    if (right.label === 'you' && left.label !== 'you') return 1;
    return 0;
  });
}

function tabFromState(name: string): Tab {
  return name === 'settings' || name === 'danger' || name === 'rekey-menu'
    ? 'settings'
    : 'people';
}

function GroupMark({
  store,
  size = 'md',
}: {
  store: Store;
  size?: 'md' | 'big';
}): ReactNode {
  return (
    <span
      className={`kico ${size} group`}
      style={{
        background:
          store.kind === 'team' && store.active === false
            ? 'var(--c-none)'
            : hue(store.name),
      }}
    >
      {store.name.slice(0, 1)}
    </span>
  );
}

function useCopyText(
  bridge: Bridge,
  onError: (error: unknown) => void,
): (text: string, message: string) => Promise<void> {
  const toasts = useToast();
  return async (text, message) => {
    try {
      await bridge.copyText(text);
      toasts.show(message);
    } catch (error) {
      onError(error);
    }
  };
}

/**
 * What discovery needs to run for one account, resolved from its exact
 * identity.
 *
 * `Account.store` is the identity; the alias is profile-local and two
 * profiles can each hold `personal`, so resolving by alias would sooner or
 * later authenticate the wrong account. Every relationship is re-checked
 * here and any inconsistency fails closed, because the caller is about to
 * write durable local bindings with whatever this returns.
 */
export interface DiscoveryContext {
  account: Account;
  store: AccountStore;
  server: Server;
  /** Discovery reads the server, so a stopped server cannot be checked. */
  available: boolean;
}

export function discoveryContext(
  world: World,
  ref: StoreRef | null,
): DiscoveryContext | null {
  if (!ref) return null;
  const account = world.accounts.find((candidate) => candidate.store === ref);
  if (!account) return null;
  const store = world.stores.find(
    (candidate) => candidate.id === account.store,
  );
  if (!store || store.kind !== 'account') return null;
  if (account.alias !== store.account || account.server !== store.server)
    return null;
  const server = world.servers.find(
    (candidate) => candidate.id === store.server,
  );
  if (!server) return null;
  return { account, store, server, available: storeReadable(world, store.id) };
}

/** One accessible name per button, since several read "Check for groups". */
export const checkLabel = (context: DiscoveryContext): string =>
  `Check for groups accessible to ${context.account.username} on ${context.server.name}`;

export const unavailableTitle = (context: DiscoveryContext): string =>
  `Restore access to ${context.server.name} before checking for groups.`;

function partySubtitle(party: Party): ReactNode {
  if (party.party_kind !== 'user') {
    const host = party.team_name?.includes(' @ ')
      ? party.team_name.split(' @ ')[1]
      : 'another server';
    return (
      <>
        group on {host}
        {party.scoped_host_id_hex ? (
          <>
            {' '}
            · host <code>{party.scoped_host_id_hex}</code>
          </>
        ) : null}
      </>
    );
  }
  const machine = Boolean(party.note?.includes('service account'));
  return `${machine ? 'machine' : 'person'} · generation ${party.generation}`;
}

function activateRow(
  event: KeyboardEvent<HTMLElement>,
  action: () => void,
): void {
  if (event.target !== event.currentTarget) return;
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    action();
  }
}

function PartyRow({
  world,
  party,
  selected,
  onSelect,
  onOpen,
  manageable,
}: {
  world: World;
  party: Party;
  selected: boolean;
  onSelect: () => void;
  onOpen: () => void;
  manageable: boolean;
}): ReactNode {
  const active = admissionActive(world, party, party.store);
  const actionable = manageable && canTarget(world, party);
  return (
    <div
      className={`prow${selected ? ' sel' : ''}${active ? '' : ' dim'}`}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={onSelect}
      onKeyDown={(event) => activateRow(event, onSelect)}
    >
      <span className="who2">
        <span className="t">
          <b>
            <span>{partyShortName(party)}</span>
            {party.label ? <Chip tone="you">you</Chip> : null}
          </b>
          <small>{partySubtitle(party)}</small>
        </span>
      </span>
      <RoleChip
        role={party.destination_role}
        extra={
          [
            party.party_kind !== 'user'
              ? `${fmtRole(party.source_role)} at ${partyShortName(party)}`
              : null,
            // Inactive memberships do not grant access.
            active ? null : 'Access paused',
          ]
            .filter(Boolean)
            .join(' · ') || undefined
        }
      />
      <span className="acts">
        {actionable ? (
          <button
            type="button"
            title="Change role or remove"
            aria-label="Change role or remove"
            onClick={(event) => {
              event.stopPropagation();
              onOpen();
            }}
          >
            <Icon name="more" />
          </button>
        ) : null}
      </span>
    </div>
  );
}

function SituationBand({
  store,
  tab,
  onFinish,
}: {
  store: Store;
  tab: Tab;
  onFinish: () => void;
}): ReactNode {
  if (store.kind === 'team' && store.active === false) {
    return (
      <Band label="Setup incomplete.">
        Group members and items are unavailable until setup is finished.{' '}
        <button type="button" className="lnk" onClick={onFinish}>
          Finish setup
        </button>
      </Band>
    );
  }
  if (
    store.kind === 'team' &&
    store.team_kind === 'adhoc' &&
    tab !== 'settings'
  ) {
    return (
      <Band severity="info" label="Ad-hoc group">
        Membership is fixed when it’s created; people can’t be added or removed
        here. Permissions and items function normally.
      </Band>
    );
  }
  return null;
}

function PeopleTab({
  world,
  store,
  selected,
  onSelect,
  onSheet,
  onFinish,
  manageable,
  failure,
  onRetry,
}: {
  world: World;
  store: Store;
  selected: Party | null;
  onSelect: (party: Party | null) => void;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onFinish: () => void;
  manageable: boolean;
  failure?: GroupDetailFailure;
  onRetry: () => void;
}): ReactNode {
  const parties = sortRoster(partiesOf(world, store.id));
  const inactive = store.kind === 'team' && store.active === false;
  const showAddActions = store.kind === 'team' && store.team_kind === 'named';
  const unavailableHint = manageable
    ? undefined
    : 'Not available for this group';
  return (
    <div className="roster">
      <SituationBand store={store} tab="people" onFinish={onFinish} />
      {failure ? (
        <Notice
          severity="warn"
          title="Roster unavailable"
          actions={
            failure.retryable ? (
              <Button onClick={onRetry}>Refresh</Button>
            ) : undefined
          }
        >
          <p>{failure.message}</p>
        </Notice>
      ) : inactive ? null : (
        <>
          <div className="rhead">
            <h2>People &amp; groups</h2>
            <span className="n">{peopleGroups(parties)}</span>
            {showAddActions ? (
              <div className="right">
                <Button
                  disabled={!manageable}
                  title={
                    manageable
                      ? 'Give every member of another group a role here'
                      : unavailableHint
                  }
                  icon="people"
                  onClick={() => onSheet('admit')}
                >
                  Add a group
                </Button>
                <Button
                  disabled={!manageable}
                  title={
                    manageable
                      ? 'Add someone who already has an account on this server'
                      : unavailableHint
                  }
                  icon="plus"
                  onClick={() => onSheet('add')}
                >
                  Add someone
                </Button>
              </div>
            ) : null}
          </div>
          {parties.length ? (
            <div className="rt">
              <div className="hdr">
                <span>Who</span>
                <span>Role</span>
                <span />
              </div>
              {parties.map((party) => (
                <PartyRow
                  key={party.party_id_hex}
                  world={world}
                  party={party}
                  selected={selected?.party_id_hex === party.party_id_hex}
                  onSelect={() =>
                    onSelect(
                      selected?.party_id_hex === party.party_id_hex
                        ? null
                        : party,
                    )
                  }
                  onOpen={() => onSelect(party)}
                  manageable={manageable}
                />
              ))}
            </div>
          ) : (
            <div className="callout">
              <span
                className="kico"
                style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
              >
                <Icon name="people" />
              </span>
              <span className="t">
                <b>No people yet.</b>
                {showAddActions
                  ? ' Add someone by their username on this server, or admit another group.'
                  : null}
              </span>
              {manageable ? (
                <Button onClick={() => onSheet('add')}>Add someone</Button>
              ) : null}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function PartyPanel({
  world,
  party,
  onClose,
  onSheet,
  manageable,
}: {
  world: World;
  party: Party;
  onClose: () => void;
  onSheet: (sheet: Sheet, party?: Party) => void;
  manageable: boolean;
}): ReactNode {
  const items = itemsOf(world, party.store);
  const readable = new Set(readsOf(world, party));
  const machine =
    party.party_kind === 'user' &&
    Boolean(party.note?.includes('service account'));
  const here = roleText(party);
  const active = admissionActive(world, party, party.store);
  const group = storeOf(world, party.store);
  const pinned = party.scoped_host_id_hex
    ? world.servers.find(
        (server) =>
          server.host_id && server.host_id === party.scoped_host_id_hex,
      )
    : undefined;
  const kind = party.party_kind === 'user' ? 'person' : 'group';
  return (
    <aside className="details">
      <div className="dh">
        <span className="t">
          <h2>
            {partyShortName(party)}
            {party.label ? <Chip tone="you">you</Chip> : null}
          </h2>
          <small>
            {kind} · {here} here
          </small>
        </span>
        <button
          type="button"
          className="x"
          title="Close"
          aria-label="Close"
          onClick={onClose}
        >
          <Icon name="x" />
        </button>
      </div>
      <div className="scroll">
        <SectionLabel
          action={
            <span className="pv">
              · {readable.size} of {items.length}
            </span>
          }
        >
          Can read
        </SectionLabel>
        {items.length ? (
          <div className="rlist">
            {items.map((item) => {
              const kindName = kindOf(item);
              return (
                <div
                  className={`ir${readable.has(item) ? '' : ' no'}`}
                  key={item.path}
                >
                  {kindName === 'Folder' ? null : <KindIcon kind={kindName} />}
                  <span className="ipth">{item.path}</span>
                  <span className="pchip">{fmtRole(item.read)}</span>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="hint">No items in {group?.name} yet.</p>
        )}
        {!active ? (
          <p className="hint">
            This group connection is inactive. Members cannot access items until
            the connection is renewed.
          </p>
        ) : readable.size < items.length ? (
          <p className="hint">
            Items you don't have permission to view are dimmed.
          </p>
        ) : null}
        <SectionLabel>Details</SectionLabel>
        <div className="meta">
          <b>Role here</b>
          <span>{here}</span>
          {party.party_kind !== 'user' ? (
            <>
              <b>At source</b>
              <span>
                {fmtRole(party.source_role)} in {partyShortName(party)}
              </span>
              <b>Host</b>
              <code>{party.scoped_host_id_hex}</code>
              {pinned ? (
                <>
                  <b>Pin</b>
                  <span>matches this Mac’s pin</span>
                </>
              ) : party.scoped_host_id_hex ? (
                <>
                  <b>Pin</b>
                  <span>Server pin not verified</span>
                </>
              ) : null}
            </>
          ) : null}
          <b>Managed</b>
          <span>
            {party.locally_manageable
              ? 'from this server'
              : 'from its own group'}
          </span>
        </div>
        {machine ? (
          <p className="hint">
            {party.note?.[0]?.toUpperCase()}
            {party.note?.slice(1)}.
          </p>
        ) : null}
      </div>
      {manageable && canTarget(world, party) ? (
        <div className="dfoot">
          {demotionFor(party) ? (
            <Button onClick={() => onSheet('demote', party)}>
              Lower role…
            </Button>
          ) : null}
          <Button variant="danger" onClick={() => onSheet('remove', party)}>
            Remove…
          </Button>
        </div>
      ) : null}
    </aside>
  );
}

function FederationSection({
  world,
  store,
  onSheet,
  onRerun,
  onExpel,
  manageable,
  failure,
  onRetry,
}: {
  world: World;
  store: Store;
  onSheet: (sheet: Sheet) => void;
  onRerun: (operationId: string) => void;
  onExpel: (entry: FederationEntry) => void;
  manageable: boolean;
  failure?: GroupDetailFailure;
  onRetry: () => void;
}): ReactNode {
  const entries = world.federation.filter((entry) => entry.store === store.id);
  const inactive = store.kind === 'team' && store.active === false;
  const parties = partiesOf(world, store.id);
  const showAddAction = store.kind === 'team' && store.team_kind === 'named';
  return (
    <div className="roster federation-section">
      {failure ? (
        <Notice
          severity="warn"
          title="Federation unavailable"
          actions={
            failure.retryable ? (
              <Button onClick={onRetry}>Refresh</Button>
            ) : undefined
          }
        >
          <p>{failure.message}</p>
        </Notice>
      ) : inactive ? null : (
        <>
          <div className="rhead">
            <h2>Groups on other servers</h2>
            {showAddAction ? (
              <div className="right">
                <Button
                  icon="plus"
                  disabled={!manageable}
                  title={
                    manageable ? undefined : 'Not available for this group'
                  }
                  onClick={() => onSheet('admit')}
                >
                  Add a group
                </Button>
              </div>
            ) : null}
          </div>
          {entries.length ? (
            <div className="rt fed">
              <div className="hdr">
                <span>Group</span>
                <span>Role here</span>
                <span>Admission</span>
                <span>Operation</span>
                <span />
              </div>
              {entries.map((entry) => {
                const party = parties.find(
                  (candidate) =>
                    candidate.party_id_hex === entry.remote_team_id_hex,
                );
                const remoteName =
                  world.servers.find(
                    (server) => server.id === entry.remote_profile,
                  )?.name ?? entry.remote_profile;
                const readable = party ? readsOf(world, party).length : 0;
                return (
                  <div
                    className="prow"
                    key={`${entry.remote_host_id_hex}|${entry.remote_team_id_hex}`}
                  >
                    <span className="who2">
                      <span className="t">
                        <b>
                          <span>{entry.remote_team_alias}</span>
                        </b>
                        <small>
                          on {remoteName} · host{' '}
                          <code>{entry.remote_host_id_hex}</code>
                        </small>
                      </span>
                    </span>
                    <RoleChip
                      role={entry.destination}
                      extra="for every member"
                    />
                    <span className="reads">
                      <Chip tone={entry.active ? 'ok' : 'warn'}>
                        {entry.active ? 'Active' : 'Inactive'}
                      </Chip>
                      <small>
                        {entry.active
                          ? `${plural(readable, 'item')} readable`
                          : 'members read nothing here'}
                      </small>
                    </span>
                    <span className="vern">{entry.operation_id_hex}</span>
                    <span className="cellact">
                      {!entry.active && entry.operation_id_hex ? (
                        <Button
                          size="sm"
                          icon="again"
                          disabled={!manageable}
                          onClick={() => onRerun(entry.operation_id_hex!)}
                        >
                          Restore access
                        </Button>
                      ) : entry.active ? (
                        <Button
                          size="sm"
                          variant="danger"
                          disabled={!manageable}
                          onClick={() => onExpel(entry)}
                        >
                          Remove…
                        </Button>
                      ) : null}
                    </span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="callout">
              <span
                className="kico"
                style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
              >
                <Icon name="people" />
              </span>
              <span className="t">
                <b>No groups from other servers.</b>
                {showAddAction
                  ? ' Admitting a group gives every one of its members the same role here.'
                  : null}
              </span>
              {manageable ? (
                <Button onClick={() => onSheet('admit')}>Add a group</Button>
              ) : null}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function FederationExpulsionSheet({
  bridge,
  store,
  entry,
  onClose,
  onApplied,
  onMutationError,
}: {
  bridge: Bridge;
  store: Extract<Store, { kind: 'team' }>;
  entry: FederationEntry;
  onClose: () => void;
  onApplied: (message: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const expel = async (): Promise<void> => {
    if (!confirmed || busy || !entry.active) return;
    setBusy(true);
    try {
      await bridge.expelFederatedGroup({
        storeId: store.id,
        remoteHostIdHex: entry.remote_host_id_hex,
        remoteTeamIdHex: entry.remote_team_id_hex,
      });
      await onApplied(
        `${entry.remote_team_alias} removed and group keys rotated`,
      );
      onClose();
    } catch (error) {
      await onMutationError(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger
      dismissible={false}
      onClose={onClose}
      glyph={<GroupMark store={store} />}
      title={`Expel ${entry.remote_team_alias} from ${store.name}?`}
      subtitle="This targets the exact remote group and server shown below"
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={busy || !confirmed || !entry.active}
            onClick={() => void expel()}
          >
            Expel and rekey
          </Button>
        </>
      }
    >
      <p>
        Every member of <b>{entry.remote_team_alias}</b> will lose access to
        this group. Group encryption keys will be updated to block future
        access.
      </p>
      <Inset>
        <InsetRow label="Remote host">
          <code>{entry.remote_host_id_hex}</code>
        </InsetRow>
        <InsetRow label="Remote group">
          <code>{entry.remote_team_id_hex}</code>
        </InsetRow>
      </Inset>
      <label className="checkline">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        I understand this expels every member of the remote group and rotates
        affected keys.
      </label>
    </SheetDialog>
  );
}

function SettingsTab({
  world,
  store,
  onSheet,
  onNavigate,
  onCopy,
  onFinish,
  manageable,
  rekeyOpen,
}: {
  world: World;
  store: Extract<Store, { kind: 'team' }>;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onFinish: () => void;
  manageable: boolean;
  rekeyOpen: boolean;
}): ReactNode {
  const parties = partiesOf(world, store.id);
  const mine = parties.find((party) => party.label === 'you');
  const owner = parties.find(
    (party) => parseRole(party.destination_role)?.kind === 'owner',
  );
  const account = world.accounts.find(
    (candidate) =>
      candidate.alias === store.account && candidate.server === store.server,
  );
  const server = serverOf(world, store.id);
  const seniors = parties
    .filter(
      (party) =>
        party.party_kind === 'user' &&
        party.label !== 'you' &&
        roleRank(party.destination_role) >= 2,
    )
    .map((party) => partyShortName(party));
  const removable = manageable
    ? parties
        .map((party, index) => ({ party, index }))
        .filter(({ party }) => canTarget(world, party))
        .sort((left, right) => {
          const rank =
            roleRank(left.party.destination_role) -
            roleRank(right.party.destination_role);
          if (rank) return rank;
          const leftRole = parseRole(left.party.destination_role);
          const rightRole = parseRole(right.party.destination_role);
          const vis =
            (leftRole?.kind === 'member' ? visibilityOf(leftRole) : 0) -
            (rightRole?.kind === 'member' ? visibilityOf(rightRole) : 0);
          if (vis) return vis;
          return left.index - right.index;
        })
        .map(({ party }) => party)
    : [];
  const total = itemsOf(world, store.id).length;
  return (
    <div className="group-settings">
      <SituationBand store={store} tab="settings" onFinish={onFinish} />
      <SectionLabel>About this group</SectionLabel>
      <Inset>
        <InsetRow label="Name">{store.name}</InsetRow>
        <InsetRow
          label="Server"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Open server
            </Button>
          }
        >
          {server?.name}
        </InsetRow>
        <InsetRow label="Your account">
          {account?.username ?? store.account} · {mine ? roleText(mine) : '—'}
        </InsetRow>
        <InsetRow label="Owner">
          {owner ? partyShortName(owner) : 'No owner designated'}
        </InsetRow>
        <InsetRow
          label="Group ID"
          action={
            <Button
              size="sm"
              icon="copy"
              onClick={() => onCopy(store.team_id_hex)}
            >
              Copy
            </Button>
          }
        >
          <code>{store.team_id_hex}</code>
        </InsetRow>
      </Inset>
      {store.team_kind === 'adhoc' ? (
        <p className="fn">
          An ad-hoc group has no name on the server and a fixed membership.
        </p>
      ) : null}
      <SectionLabel>Danger</SectionLabel>
      <Inset className="danger settings-inset">
        <InsetRow
          action={
            <MenuButton
              variant="danger"
              label="Remove…"
              menuLabel="Removable members"
              disabled={!removable.length}
              defaultOpen={rekeyOpen}
            >
              {(close) =>
                removable.map((party) => (
                  <button
                    type="button"
                    key={party.party_id_hex}
                    onClick={() => {
                      close();
                      onSheet('remove', party);
                    }}
                  >
                    <span className="t">
                      <b>{partyShortName(party)}</b>
                      <small>
                        {fmtRole(party.destination_role)} · reads{' '}
                        {readsOf(world, party).length} of {total}
                      </small>
                    </span>
                  </button>
                ))
              }
            </MenuButton>
          }
        >
          <span className="t">
            <b>Remove a group member</b>
            <small>
              Removing anyone rotates the group key and blocks their future
              reads. Copies already downloaded are not erased.
            </small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button variant="danger" disabled>
              Leave…
            </Button>
          }
        >
          <span className="t">
            <b>Leave {store.name}</b>
            <small>
              {seniors.length
                ? `To leave this group, ask ${oxfordOr(seniors)} to remove your account.`
                : 'As the sole owner, you must transfer ownership or delete the group to leave.'}
            </small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button variant="danger" disabled>
              Delete…
            </Button>
          }
        >
          <span className="t">
            <b>Delete {store.name}</b>
            <small>
              Permanently deletes the group and all shared items for all
              members.
            </small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button
              icon="out"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Servers
            </Button>
          }
        >
          <span className="t">
            <b>Reset this Mac’s state for {server?.name}</b>
            <small>
              Removes locally stored keys and server data from this Mac.
            </small>
          </span>
        </InsetRow>
      </Inset>
    </div>
  );
}

function inspectResponse(world: World, store: Store, tab: Tab): unknown {
  if (tab === 'settings') return store;
  return {
    people: partiesOf(world, store.id).map((party) => ({
      ...(party.username ? { username: party.username } : {}),
      party_kind: party.party_kind,
      generation: party.generation,
      locally_manageable: party.locally_manageable,
      party_id_hex: party.party_id_hex,
      ...(party.scoped_host_id_hex
        ? { scoped_host_id_hex: party.scoped_host_id_hex }
        : {}),
      source_role: party.source_role,
      destination_role: party.destination_role,
    })),
    federation: world.federation
      .filter((entry) => entry.store === store.id)
      .map((entry) => ({
        remote_profile: entry.remote_profile,
        remote_team_alias: entry.remote_team_alias,
        remote_host_id_hex: entry.remote_host_id_hex,
        remote_team_id_hex: entry.remote_team_id_hex,
        destination: entry.destination,
        ...(entry.operation_id_hex
          ? { operation_id_hex: entry.operation_id_hex }
          : {}),
        active: entry.active,
      })),
  };
}

export function GroupSheet({
  world,
  bridge,
  store,
  sheet,
  target,
  onClose,
  onSwitch,
  onApplied,
  onError,
  onMutationError,
}: {
  world: World;
  bridge: Bridge;
  store: Store;
  sheet: Exclude<Sheet, null>;
  target: Party | null;
  onClose: () => void;
  onSwitch: (sheet: Sheet, target?: Party) => void;
  onApplied: (
    message: string,
    created?: { accountStoreId: StoreRef; teamAlias: string },
  ) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const copy = useCopyText(bridge, onError);
  const toasts = useToast();
  const [username, setUsername] = useState('jules.park');
  const [visibility, setVisibility] = useState(0);
  const callerParty = partiesOf(world, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const callerRank = callerParty ? roleRank(callerParty.destination_role) : 0;
  const [role, setRole] = useState<RoleDto>({
    role: 'Member',
    visibility: 0,
  });
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState('Platform');
  const [createKind, setCreateKind] = useState<'named' | 'adhoc'>('named');
  const creationAccounts = world.stores.filter(
    (candidate) =>
      candidate.kind === 'account' && canCreateInStore(world, candidate.id),
  );
  const [accountStoreId, setAccountStoreId] = useState(
    () =>
      creationAccounts.find((candidate) => candidate.account === 'work')?.id ??
      creationAccounts[0]?.id ??
      '',
  );
  const remotes = world.stores.filter(
    (candidate): candidate is Extract<Store, { kind: 'team' }> =>
      candidate.kind === 'team' &&
      candidate.active &&
      candidate.team_kind === 'named' &&
      storeReadable(world, candidate.id) &&
      candidate.server !== store.server,
  );
  const [remoteStoreId, setRemoteStoreId] = useState(remotes[0]?.id ?? '');
  const creationAccount = creationAccounts.find(
    (candidate) => candidate.id === accountStoreId,
  );
  const server = serverOf(
    world,
    sheet === 'create' && creationAccount ? creationAccount.id : store.id,
  );
  const ownerStore =
    store.kind === 'team'
      ? world.stores.find(
          (candidate) =>
            candidate.kind === 'account' &&
            candidate.server === store.server &&
            candidate.account === store.account,
        )
      : undefined;
  const ownerAccount = ownerStore
    ? world.accounts.find(
        (account) =>
          account.store === ownerStore.id ||
          (account.server === ownerStore.server &&
            account.alias === ownerStore.account),
      )
    : undefined;
  const inviter =
    ownerAccount?.username.split('.')[0] ??
    ownerAccount?.username ??
    'the group Admin';
  const inviteMessage = `I'd like to invite you to join ${store.name} on FOKS.\n\n1. Download FOKS: https://foks.app/download\n2. Add server: ${server?.name ?? store.server}\n3. Create your account on the server\n4. Send your username to ${inviter}\n\nOnce added, ${store.name} will appear in your Groups list.`;
  const teamAlias = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
  const remote =
    remotes.find((candidate) => candidate.id === remoteStoreId) ?? remotes[0];
  const requiredFailure =
    sheet === 'admit'
      ? groupDetailFailure(world, store.id, 'federation')
      : ['add', 'demote', 'remove'].includes(sheet)
        ? groupDetailFailure(world, store.id, 'roster')
        : undefined;
  const title =
    sheet === 'invite'
      ? `Invite someone to ${store.name}`
      : sheet === 'add'
        ? `Add someone to ${store.name}`
        : sheet === 'demote'
          ? `Lower ${target ? `${partyShortName(target)}’s` : 'their'} role`
          : sheet === 'remove'
            ? `Remove ${target ? partyName(target) : 'them'} from ${store.name}?`
            : sheet === 'admit'
              ? `Add a group to ${store.name}`
              : 'Create a group';
  const subtitle =
    sheet === 'create'
      ? (server?.name ?? 'Selected server')
      : sheet === 'invite'
        ? 'Send instructions to help a new user set up their account'
        : sheet === 'add'
          ? 'Add an existing user from this server'
          : sheet === 'demote'
            ? `${target ? roleText(target) : ''} in ${store.name} today`
            : sheet === 'remove'
              ? ''
              : sheet === 'admit'
                ? 'Every member of that group gets the same role here'
                : server?.name;
  const [demotion, setDemotion] = useState<RoleDto | null>(() =>
    target ? demotionFor(target) : null,
  );
  const currentRole = target ? parseRole(target.destination_role) : null;
  const maxMemberVisibility =
    currentRole?.kind === 'member' ? (currentRole.visibility ?? 0) - 1 : 0;
  useEffect(() => {
    if (sheet === 'demote') setDemotion(target ? demotionFor(target) : null);
  }, [sheet, target]);
  const apply = async (): Promise<void> => {
    if (busy || requiredFailure) return;
    setBusy(true);
    try {
      if (sheet === 'invite') {
        await bridge.copyText(inviteMessage);
        toasts.show('Message copied.');
      } else if (sheet === 'add')
        await bridge.addGroupMember({
          storeId: store.id,
          username: username.trim(),
          destination: role.role === 'Member' ? { ...role, visibility } : role,
        });
      else if (
        sheet === 'demote' &&
        target &&
        canTarget(world, target) &&
        demotion
      )
        await bridge.demoteGroupMember({
          storeId: store.id,
          username: target.username!,
          destination: demotion,
        });
      else if (sheet === 'remove' && target && canTarget(world, target))
        await bridge.removeGroupMember({
          storeId: store.id,
          username: target.username!,
        });
      else if (sheet === 'admit' && remote)
        await bridge.admitGroup({
          storeId: store.id,
          remoteStoreId: remote.id,
          visibility,
        });
      else if (sheet === 'create') {
        // Re-resolved from the availability-filtered list at the moment of
        // the write, not from the render that drew the button: the server
        // can lapse while the sheet is open.
        const account = creationAccounts.find(
          (candidate) => candidate.id === accountStoreId,
        );
        if (!account)
          throw new Error('No account store is available for group creation.');
        await bridge.createGroup({
          accountStoreId: account.id,
          teamAlias,
          name: createKind === 'named' ? name : '',
          kind: createKind,
        });
        await onApplied(`${title} completed`, {
          accountStoreId: account.id,
          teamAlias,
        });
        onClose();
        return;
      }
      if (sheet !== 'invite') await onApplied(`${title} completed`);
      onClose();
    } catch (error) {
      if (sheet === 'invite') onError(error);
      else await onMutationError(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger={sheet === 'remove'}
      onClose={onClose}
      dismissible={!busy}
      width={sheet === 'invite' ? 'wide' : 'base'}
      glyph={
        sheet === 'create' ? (
          <span
            className="kico md group"
            style={{ background: hue(name || 'group') }}
          >
            {(name.trim() || 'G').slice(0, 1)}
          </span>
        ) : (
          <GroupMark store={store} />
        )
      }
      title={title}
      subtitle={subtitle}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          {sheet === 'remove' ? (
            <Button
              variant="primary"
              danger
              disabled={
                busy ||
                Boolean(requiredFailure) ||
                !target ||
                !canTarget(world, target)
              }
              onClick={() => void apply()}
            >
              Remove and rekey
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={
                busy ||
                Boolean(requiredFailure) ||
                (sheet === 'add' && !username.trim()) ||
                (sheet === 'demote' &&
                  (!target || !canTarget(world, target) || !demotion)) ||
                (sheet === 'admit' && !remote) ||
                (sheet === 'create' && (!teamAlias || !creationAccount))
              }
              onClick={() => void apply()}
            >
              {sheet === 'invite'
                ? 'Copy message'
                : sheet === 'add'
                  ? `Add ${username.trim() || 'someone'}`
                  : sheet === 'demote'
                    ? 'Change role'
                    : sheet === 'admit'
                      ? `Admit ${remote?.kind === 'team' ? remote.alias : 'group'}`
                      : `Create ${name.trim() || 'group'}`}
            </Button>
          )}
        </>
      }
    >
      <>
        {requiredFailure ? (
          <Notice
            severity="warn"
            title={`${requiredFailure.source === 'roster' ? 'Roster' : 'Federation'} unavailable`}
          >
            <p>
              {requiredFailure.message} Close this sheet and refresh before
              making changes.
            </p>
          </Notice>
        ) : null}
        {sheet === 'invite' ? (
          <>
            <p>
              They need an account on {server?.name} before you can add them.
              Send this message, then add their username.
            </p>
            <Inset>
              <InsetRow label="Message">
                <span className="msg">{inviteMessage}</span>
                <Button
                  size="sm"
                  onClick={() => void copy(inviteMessage, 'Message copied.')}
                >
                  Copy
                </Button>
              </InsetRow>
            </Inset>
            <SectionLabel>Message contents</SectionLabel>
            <Inset className="rows">
              <InsetRow label="Install link">
                <code>https://foks.app/download</code>{' '}
                <Chip tone="warn">placeholder</Chip>
                <span className="hint">Temporary download address.</span>
              </InsetRow>
              <InsetRow label="Server">
                <span>
                  {server?.name}
                  <span className="hint">
                    They enter this address during setup.
                  </span>
                </span>
              </InsetRow>
              <InsetRow label="Signup invite">
                <span>
                  Optional
                  <span className="hint">
                    Required only if the server requires an invite.
                  </span>
                </span>
              </InsetRow>
              <InsetRow label="When they reply">
                <span>Add their username as a Member, Admin, or Owner.</span>
                <Button size="sm" onClick={() => onSwitch('add')}>
                  Add someone
                </Button>
              </InsetRow>
            </Inset>
            <p className="hint">
              Send this message to the person you want to invite.
            </p>
          </>
        ) : null}
        {sheet === 'add' ? (
          <>
            <Inset>
              <Field label="Username" value={username} onChange={setUsername} />
            </Inset>
            <SectionLabel>Role in {store.name}</SectionLabel>
            <Inset>
              <RadioGroup label={`Role in ${store.name}`}>
                {(callerRank >= 3
                  ? (['Member', 'Admin', 'Owner'] as const)
                  : (['Member', 'Admin'] as const)
                ).map((next) => (
                  <RadioCard
                    key={next}
                    selected={role.role === next}
                    onSelect={() =>
                      setRole(
                        next === 'Member'
                          ? { role: next, visibility }
                          : { role: next },
                      )
                    }
                    title={next}
                    detail={
                      next === 'Member'
                        ? 'Opens items at or above its visibility band. Visibility 0 is the default.'
                        : next === 'Admin'
                          ? 'Changes items and adds or removes people. Cannot change other Admins or the Owner.'
                          : 'Everything, including deleting the group.'
                    }
                  />
                ))}
              </RadioGroup>
            </Inset>
            {role.role === 'Member' ? (
              <div className="vis">
                <Button
                  size="sm"
                  disabled={visibility <= VIS_MIN}
                  onClick={() => setVisibility((value) => value - 1)}
                >
                  −
                </Button>
                <span>Visibility {visibility}</span>
                <Button
                  size="sm"
                  disabled={visibility >= VIS_MAX}
                  onClick={() => setVisibility((value) => value + 1)}
                >
                  +
                </Button>
              </div>
            ) : null}
            <p className="fn">
              They can access items allowed by their role immediately. No
              invitation is sent; {store.name} will appear when their app checks
              the server.
            </p>
          </>
        ) : null}
        {sheet === 'demote' ? (
          <>
            <p>
              To grant a higher role, remove the member and add them again with
              the updated role.
            </p>
            <Inset>
              <RadioGroup label="New role">
                {currentRole?.kind === 'owner' ? (
                  <RadioCard
                    selected={demotion?.role === 'Admin'}
                    onSelect={() => setDemotion({ role: 'Admin' })}
                    title="Admin"
                    detail="Full access to group items and permission to manage members."
                  />
                ) : null}
                <RadioCard
                  selected={demotion?.role === 'Member'}
                  disabled={maxMemberVisibility < VIS_MIN}
                  onSelect={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: maxMemberVisibility,
                    })
                  }
                  title={
                    currentRole?.kind === 'member'
                      ? `Member · visibility ${maxMemberVisibility}`
                      : 'Member'
                  }
                  detail={
                    currentRole?.kind === 'member'
                      ? `Same role, lower level; ${maxMemberVisibility} at most.`
                      : 'Cannot manage members; can only access items allowed by their member role.'
                  }
                />
                <RadioCard
                  off
                  selected={false}
                  title={
                    <>
                      {target ? roleText(target) : 'Current role'}{' '}
                      <Chip>current</Chip>
                    </>
                  }
                  detail="Current active role."
                />
              </RadioGroup>
            </Inset>
            {demotion?.role === 'Member' ? (
              <div className="vis">
                <Button
                  size="sm"
                  disabled={demotion.visibility <= VIS_MIN}
                  onClick={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: demotion.visibility - 1,
                    })
                  }
                >
                  −
                </Button>
                <span>Visibility {demotion.visibility}</span>
                <Button
                  size="sm"
                  disabled={demotion.visibility >= maxMemberVisibility}
                  onClick={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: demotion.visibility + 1,
                    })
                  }
                >
                  +
                </Button>
              </div>
            ) : null}
          </>
        ) : null}
        {sheet === 'remove' ? (
          <>
            <p>
              Removing blocks future reads and rekeys the group. This user may
              retain a local copy of their current records.
            </p>
            {target && !canTarget(world, target) ? (
              <Notice title={`${partyName(target)} cannot be removed here`}>
                This member cannot be removed here. They are managed by another
                server or account.
              </Notice>
            ) : null}
          </>
        ) : null}
        {sheet === 'admit' ? (
          <>
            <SectionLabel>Group</SectionLabel>
            <Inset>
              {remotes.length ? (
                <RadioGroup label="Group">
                  {remotes.map((group) => {
                    const host = serverOf(world, group.id);
                    return (
                      <RadioCard
                        key={group.id}
                        selected={remote?.id === group.id}
                        onSelect={() => setRemoteStoreId(group.id)}
                        title={group.alias}
                        detail={`on ${host?.name ?? group.server}`}
                      />
                    );
                  })}
                </RadioGroup>
              ) : (
                <InsetRow label="Group">
                  <span className="dim">No eligible remote group</span>
                </InsetRow>
              )}
            </Inset>
            <SectionLabel>Role for its members</SectionLabel>
            <Inset>
              <InsetRow label="Role">
                <Chip>Member</Chip>
                <span className="dim">for every member</span>
              </InsetRow>
              <InsetRow
                label="Visibility"
                action={
                  <>
                    <Button
                      size="sm"
                      disabled={visibility <= VIS_MIN}
                      onClick={() => setVisibility((value) => value - 1)}
                    >
                      −
                    </Button>
                    <Button
                      size="sm"
                      disabled={visibility >= VIS_MAX}
                      onClick={() => setVisibility((value) => value + 1)}
                    >
                      +
                    </Button>
                  </>
                }
              >
                {visibility}
              </InsetRow>
            </Inset>
            <p className="fn">
              {store.name} automatically syncs members from that group. This
              connection cannot be removed from this screen.
            </p>
          </>
        ) : null}
        {sheet === 'create' ? (
          <>
            <Inset>
              <Field label="Name" value={name} onChange={setName} />
            </Inset>
            <p className="fn">
              Others find it as <code>{teamAlias || '…'}</code> on the server.
            </p>
            <SectionLabel>Server and account</SectionLabel>
            <Inset>
              {creationAccounts.length ? (
                <RadioGroup label="Server and account">
                  {creationAccounts.map((account) => (
                    <RadioCard
                      key={account.id}
                      selected={account.id === accountStoreId}
                      onSelect={() => setAccountStoreId(account.id)}
                      title={serverOf(world, account.id)?.name}
                      detail={`as ${world.accounts.find((candidate) => candidate.store === account.id || (candidate.alias === account.account && candidate.server === account.server))?.username ?? account.account}`}
                    />
                  ))}
                </RadioGroup>
              ) : (
                <InsetRow label="Account">
                  <span className="dim">No account can create a group.</span>
                </InsetRow>
              )}
            </Inset>
            <SectionLabel>Kind</SectionLabel>
            <Inset>
              <RadioGroup label="Kind">
                {(['named', 'adhoc'] as const).map((kind) => (
                  <RadioCard
                    key={kind}
                    selected={kind === createKind}
                    onSelect={() => setCreateKind(kind)}
                    title={kind === 'named' ? 'Named' : 'Ad-hoc'}
                    detail={
                      kind === 'named'
                        ? 'Has an alias on the server; people can be added and removed over time.'
                        : 'Fixed membership, chosen now, no alias. For a one-off share.'
                    }
                  />
                ))}
              </RadioGroup>
            </Inset>
          </>
        ) : null}
      </>
    </SheetDialog>
  );
}

export function GroupSettingsScreen({
  world,
  bridge,
  location,
  onNavigate,
  onApplied: onWorldApplied,
  onError,
  onMutationError: onWorldMutationError,
}: {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'group-settings' }>;
  onNavigate: (location: Location) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const copy = useCopyText(bridge, onError);
  const initial = stateName();
  const [tab, setTab] = useState<Tab>(
    () => location.tab ?? tabFromState(initial),
  );
  const [sheet, setSheet] = useState<Sheet>(() =>
    ['invite', 'add', 'demote', 'remove', 'admit', 'party-remove'].includes(
      initial,
    )
      ? initial === 'party-remove'
        ? 'remove'
        : (initial as Sheet)
      : null,
  );
  useEffect(() => {
    setTab(location.tab ?? 'people');
  }, [location.ref, location.tab]);
  const store = storeOf(world, location.ref);
  const parties = useMemo(
    () => (store ? partiesOf(world, store.id) : []),
    [store, world],
  );
  const rosterFailure = store
    ? groupDetailFailure(world, store.id, 'roster')
    : undefined;
  const federationFailure = store
    ? groupDetailFailure(world, store.id, 'federation')
    : undefined;
  const [selected, setSelected] = useState<Party | null>(() =>
    initial === 'party'
      ? (parties.find((party) => party.username === 'deploy-bot') ?? null)
      : null,
  );
  const [expulsionTarget, setExpulsionTarget] =
    useState<FederationEntry | null>(null);
  // An interrupted member addition or role change leaves durable local state
  // that blocks every later membership mutation until it is resumed. Read the
  // account's pending operations for this group so the UI can finish it.
  const [membershipPending, setMembershipPending] = useState<
    PendingOperation[]
  >([]);
  const pendingProfile = store?.kind === 'team' ? store.server : undefined;
  const pendingAlias = store?.kind === 'team' ? store.alias : undefined;
  const loadMembershipPending = useCallback(async (): Promise<void> => {
    if (!pendingProfile || !pendingAlias) {
      setMembershipPending([]);
      return;
    }
    try {
      const rows = await enqueueProfileWork(bridge, pendingProfile, () =>
        bridge.listPendingOperations(pendingProfile),
      );
      setMembershipPending(
        rows.filter(
          (row) =>
            row.alias === pendingAlias &&
            (row.kind === 'team-member-addition' ||
              row.kind === 'team-member-edit'),
        ),
      );
    } catch {
      setMembershipPending([]);
    }
  }, [bridge, pendingAlias, pendingProfile]);
  useEffect(() => {
    void loadMembershipPending();
  }, [loadMembershipPending]);
  // Both successful changes and reconciled failures can change the durable
  // pending records. Refresh them for every membership action and manual refresh.
  const onApplied = useCallback(
    async (message: string): Promise<void> => {
      try {
        await onWorldApplied(message);
      } finally {
        await loadMembershipPending();
      }
    },
    [loadMembershipPending, onWorldApplied],
  );
  const onMutationError = useCallback<MutationFailureHandler>(
    async (error, options) => {
      try {
        await onWorldMutationError(error, options);
      } finally {
        await loadMembershipPending();
      }
    },
    [loadMembershipPending, onWorldMutationError],
  );
  const [target, setTarget] = useState<Party | null>(() => {
    if (initial === 'demote')
      return parties.find((party) => party.username === 'priya.n') ?? null;
    if (initial === 'remove')
      return parties.find((party) => party.username === 'dana.okafor') ?? null;
    if (initial === 'party-remove') {
      const party = parties.find(
        (candidate) => candidate.username === 'deploy-bot',
      );
      return party ? { ...party, locally_manageable: false } : null;
    }
    return null;
  });
  // `selected` and `target` are snapshots taken when a row was clicked, and a
  // refresh replaces the roster underneath them. Re-resolved here so the
  // panel cannot keep showing a role the write just changed, and a role
  // sheet cannot draft a demotion from a role the party no longer holds.
  // `target` keeps its snapshot's manageability, which a scene may force.
  const freshOf = (party: Party | null): Party | undefined =>
    party
      ? parties.find(
          (candidate) => candidate.party_id_hex === party.party_id_hex,
        )
      : undefined;
  const selectedParty = rosterFailure ? null : (freshOf(selected) ?? null);
  const targetFresh = freshOf(target);
  const targetParty = target
    ? targetFresh
      ? {
          ...target,
          source_role: targetFresh.source_role,
          destination_role: targetFresh.destination_role,
          generation: targetFresh.generation,
        }
      : target
    : null;
  const expulsionEntry = expulsionTarget
    ? (world.federation.find(
        (entry) =>
          entry.store === expulsionTarget.store &&
          entry.active &&
          entry.remote_host_id_hex === expulsionTarget.remote_host_id_hex &&
          entry.remote_team_id_hex === expulsionTarget.remote_team_id_hex,
      ) ?? null)
    : null;
  const openSheet = (next: Sheet, party?: Party): void => {
    setTarget(party ?? null);
    setSheet(next);
  };
  const peopleCount = useMemo(
    () =>
      store
        ? parties.length +
          world.federation.filter((entry) => entry.store === store.id).length
        : 0,
    [parties, store, world.federation],
  );
  const storeId = store?.id;
  const seenStore = useRef(storeId);
  const [rekeyArmed, setRekeyArmed] = useState(initial === 'rekey-menu');
  useEffect(() => {
    if (seenStore.current === storeId) return;
    if (seenStore.current && storeId && seenStore.current !== storeId) {
      setTab('people');
      setSelected(null);
      setSheet(null);
      setTarget(null);
      setExpulsionTarget(null);
      setRekeyArmed(false);
    }
    if (seenStore.current && !storeId) {
      setTab('people');
      setSelected(null);
      setSheet(null);
      setTarget(null);
      setExpulsionTarget(null);
      setRekeyArmed(false);
    }
    seenStore.current = storeId;
  }, [storeId]);
  const mutate = async (
    action: () => Promise<unknown>,
    message: string,
  ): Promise<void> => {
    try {
      await action();
      await onApplied(message);
    } catch (error) {
      await onMutationError(error);
    }
  };
  const resumeMembership = (operation: PendingOperation): void => {
    if (!store || store.kind !== 'team') return;
    if (operation.kind === 'team-member-addition') {
      const username = operation.target;
      if (!username) return;
      void mutate(
        () => bridge.resumeGroupMemberAddition({ storeId: store.id, username }),
        'Member addition resumed',
      );
      return;
    }
    if (operation.kind === 'team-member-edit') {
      void mutate(
        () => bridge.resumeGroupMemberEdit(store.id),
        'Member change resumed',
      );
    }
  };

  if (!store || store.kind !== 'team') {
    return (
      <>
        <PageHeader title="Group unavailable" subtitle="" />
        <div className="body">
          <Notice title="This group is no longer available">
            Refresh the catalog or choose another group from the sidebar.
          </Notice>
        </div>
      </>
    );
  }
  const access = storeDescriptionState(world, store);
  const unavailable = access !== 'normal' && access !== 'setup-incomplete';
  const inactive = store.active === false;
  const callerParty = partiesOf(world, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const callerRank = callerParty ? roleRank(callerParty.destination_role) : 0;
  const manageable =
    store.team_kind === 'named' && !inactive && !unavailable && callerRank >= 2;
  const rosterManageable = manageable && !rosterFailure;
  const federationManageable = manageable && !federationFailure;
  const finishSetup = (): void => {
    void mutate(
      () => bridge.resumeGroupCreation(store.id),
      'Group creation resumed',
    );
  };
  return (
    <>
      <div className="ghero">
        <button
          type="button"
          className="back"
          title={`Back to ${store.name}`}
          aria-label={`Back to ${store.name}`}
          onClick={() => onNavigate({ kind: 'store', ref: store.id })}
        >
          <Icon name="chev" />
        </button>
        <GroupMark store={store} size="big" />
        <div className="t">
          <h1>
            <span>{store.name}</span>
            {inactive ? <Chip tone="warn">Inactive</Chip> : null}
          </h1>
          <div className="sub">Group settings</div>
        </div>
        <div className="header-action">
          {unavailable ? (
            <Button
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              {access === 'check-in-expired' ? 'Open server' : 'Review server'}
            </Button>
          ) : null}
          {inactive ? null : (
            <MenuButton
              variant="quiet"
              icon="more"
              trailingIcon={null}
              label=""
              menuLabel="Group actions"
              title="More"
              aria-label="More"
            >
              {(close) => (
                <>
                  <button
                    type="button"
                    onClick={() => {
                      close();
                      void mutate(() => Promise.resolve(), 'Group refreshed');
                    }}
                  >
                    <Icon name="again" />
                    Refresh group
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      close();
                      void copy(store.team_id_hex, 'Group ID copied.');
                    }}
                  >
                    <Icon name="copy" />
                    Copy group ID
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      close();
                      onNavigate({ kind: 'store', ref: store.id });
                    }}
                  >
                    <Icon name="out" />
                    Open in vault
                  </button>
                </>
              )}
            </MenuButton>
          )}
        </div>
      </div>
      {unavailable ? (
        <StoreAccessTakeover
          world={world}
          store={store}
          noHeader
          onOpenServer={(profile) =>
            onNavigate({ kind: 'settings', section: 'servers', profile })
          }
          onFinishSetup={finishSetup}
        />
      ) : (
        <>
          {membershipPending.length ? (
            <Notice severity="warn" title="Finish a pending membership change">
              <p>
                FOKS stopped partway through changing this group’s members.
                Finish the pending change before adding, removing, or changing
                anyone else.
              </p>
              {membershipPending.map((operation) => (
                <p key={`${operation.kind}:${operation.target ?? ''}`}>
                  {operation.kind === 'team-member-addition'
                    ? `Add ${operation.target ?? 'member'}`
                    : 'Finish the role change'}{' '}
                  <Button
                    size="sm"
                    variant="primary"
                    aria-label={
                      operation.kind === 'team-member-addition'
                        ? `Resume adding ${operation.target ?? 'member'}`
                        : 'Resume the role change'
                    }
                    onClick={() => resumeMembership(operation)}
                  >
                    Resume
                  </Button>
                </p>
              ))}
            </Notice>
          ) : null}
          <Tabs
            label="Group sections"
            value={tab}
            onChange={(next) => {
              setTab(next);
              setSelected(null);
              onNavigate({ kind: 'group-settings', ref: store.id, tab: next });
            }}
            items={[
              {
                id: 'people',
                label: 'People',
                ...(rosterFailure || federationFailure
                  ? {}
                  : { count: peopleCount }),
              },
              { id: 'settings', label: 'Settings' },
            ]}
          />
          <div className={`body${selected ? ' group-panel-open' : ''}`}>
            <div className="groups-wrap">
              {tab === 'people' ? (
                <>
                  <PeopleTab
                    world={world}
                    store={store}
                    selected={selected}
                    onSelect={setSelected}
                    onSheet={openSheet}
                    onFinish={finishSetup}
                    manageable={rosterManageable}
                    failure={rosterFailure}
                    onRetry={() => void onApplied('Refreshing group members…')}
                  />
                  {rosterManageable && store.kind === 'team' && (
                    <InvitationPanel
                      bridge={bridge}
                      profile={store.server}
                      account={store.account}
                      teamAlias={store.alias}
                      onComplete={() => onApplied('Group requests updated')}
                    />
                  )}
                  <FederationSection
                    world={world}
                    store={store}
                    onSheet={openSheet}
                    onRerun={(operationId) =>
                      void mutate(
                        () => bridge.rerunGroupAdmission(store.id, operationId),
                        'Group access restored',
                      )
                    }
                    onExpel={setExpulsionTarget}
                    manageable={federationManageable}
                    failure={federationFailure}
                    onRetry={() =>
                      void onApplied('Refreshing external groups…')
                    }
                  />
                </>
              ) : (
                <SettingsTab
                  world={world}
                  store={store}
                  onSheet={openSheet}
                  onNavigate={onNavigate}
                  onCopy={(text) => void copy(text, 'Group ID copied.')}
                  onFinish={finishSetup}
                  manageable={rosterManageable}
                  rekeyOpen={rekeyArmed}
                />
              )}
              {tab === 'settings' || (!rosterFailure && !federationFailure) ? (
                <Toggle label="Inspect response">
                  <pre>
                    {JSON.stringify(
                      inspectResponse(world, store, tab),
                      null,
                      1,
                    )}
                  </pre>
                </Toggle>
              ) : null}
            </div>
          </div>
          {selectedParty ? (
            <PartyPanel
              world={world}
              party={selectedParty}
              onClose={() => setSelected(null)}
              onSheet={openSheet}
              manageable={rosterManageable}
            />
          ) : null}
          {expulsionEntry ? (
            <FederationExpulsionSheet
              bridge={bridge}
              store={store}
              entry={expulsionEntry}
              onClose={() => setExpulsionTarget(null)}
              onApplied={onApplied}
              onMutationError={onMutationError}
            />
          ) : null}
          {sheet ? (
            <GroupSheet
              world={world}
              bridge={bridge}
              store={store}
              sheet={sheet}
              target={targetParty}
              onClose={() => setSheet(null)}
              onSwitch={openSheet}
              onApplied={onApplied}
              onError={onError}
              onMutationError={onMutationError}
            />
          ) : null}
        </>
      )}
    </>
  );
}
