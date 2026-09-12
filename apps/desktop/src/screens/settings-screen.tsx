import { BotPanel } from '../components/bot-panel';
import { RenamePanel } from '../components/rename-panel';
import { SsoPanel } from '../components/sso-panel';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import {
  enqueueProfileWork,
  normalizeCommandError,
  sharedServerStatus,
  shouldReportPassiveServerStatusError,
} from '../bridge';
import type {
  AccountDevice,
  AppInfo,
  BackupEnrollment,
  Bridge,
  PairingOffer,
  ServerStatusSnapshot,
  YubiCommand,
  YubiEnrollment,
} from '../bridge';
import {
  Band,
  Button,
  Chip,
  Field,
  Icon,
  Inset,
  InsetRow,
  Notice,
  SectionLabel,
  SegmentedControl,
  SheetDialog,
  Stack,
} from '../components';
import type { Location, SettingsSection } from '../location';
import {
  canCreateInStore,
  partiesOf,
  serverLeaseState,
  serverOf,
  storeDescription,
  storeDescriptionState,
} from '../model';
import type { AccountStore, StoreRef, TeamStore, World } from '../model';
import { PageHeader } from '../shell/page-header';
import type { MutationFailureHandler } from '../mutation-recovery';
import {
  checkLabel,
  discoveryContext,
  GroupSheet,
  unavailableTitle,
} from './groups-screen';
import type { DiscoveryContext } from './groups-screen';
import { GoProfileConnectSheet } from './go-profile-connect';
import { ServersSection } from './servers-screen';

interface Props {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'settings' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshWorld: () => Promise<World>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onLock: () => Promise<boolean>;
}

type Sheet =
  | 'phrase'
  | 'pair'
  | 'recover'
  | 'enrol'
  | 'provision'
  | 'yubi'
  | 'revoke'
  | 'passphrase'
  | 'remove-device'
  | 'revoke-backup'
  | 'go-profile'
  | null;
const SECTIONS: readonly {
  id: SettingsSection;
  label: string;
  icon: 'file' | 'key' | 'vault' | 'server' | 'people' | 'info';
}[] = [
  { id: 'macs', label: 'Recovery devices', icon: 'file' },
  { id: 'keys', label: 'Security keys', icon: 'key' },
  { id: 'account', label: 'Accounts', icon: 'vault' },
  { id: 'servers', label: 'Servers', icon: 'server' },
  { id: 'groups', label: 'Groups', icon: 'people' },
  { id: 'about', label: 'About', icon: 'info' },
];

function isCatalogRequired(error: unknown): boolean {
  return normalizeCommandError(error).code === 'catalog-required';
}

function accountStores(world: World): AccountStore[] {
  return world.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
}

function GroupsSection({
  world,
  discovering,
  onDiscover,
  onCreate,
  onInvite,
  onOpen,
}: {
  world: World;
  discovering: StoreRef | null;
  onDiscover: (context: DiscoveryContext) => Promise<void>;
  onCreate: () => void;
  onInvite: (store: AccountStore) => void;
  onOpen: (store: TeamStore) => void;
}): ReactNode {
  const accounts = accountStores(world);
  const canCreate = accounts.some((store) => canCreateInStore(world, store.id));
  const groups = world.stores.filter(
    (store): store is TeamStore => store.kind === 'team',
  );
  const attention = groups.filter(
    (store) =>
      storeDescriptionState(world, store) !== 'normal' ||
      world.groupDetailFailures.some((failure) => failure.store === store.id),
  );
  return (
    <>
      <SectionLabel>Create a group</SectionLabel>
      <Inset className="settings-inset middle">
        <InsetRow
          label="New group"
          action={
            <Button
              size="sm"
              icon="plus"
              variant="primary"
              disabled={!canCreate}
              title={
                canCreate
                  ? undefined
                  : 'No available account can create a group'
              }
              onClick={onCreate}
            >
              Create group…
            </Button>
          }
        />
      </Inset>
      {canCreate ? null : (
        <p className="fn">
          No available account can create a group right now. Add an account, or
          restore access to a server.
        </p>
      )}
      <SectionLabel>Find groups</SectionLabel>
      <Inset className="settings-inset">
        {accounts.length ? (
          accounts.map((store) => {
            const context = discoveryContext(world, store.id);
            const busy = discovering === store.id;
            const account = world.accounts.find(
              (candidate) => candidate.store === store.id,
            );
            return (
              <InsetRow
                key={store.id}
                label={context?.server.name ?? store.name}
                action={
                  <Button
                    size="sm"
                    aria-label={
                      context ? checkLabel(context) : 'Check for groups'
                    }
                    title={
                      context && !context.available
                        ? unavailableTitle(context)
                        : 'Ask the server which groups this account belongs to.'
                    }
                    disabled={!context || !context.available || busy}
                    onClick={() => {
                      if (context) void onDiscover(context);
                    }}
                  >
                    {busy ? 'Checking…' : 'Check for groups'}
                  </Button>
                }
              >
                <small>
                  {account
                    ? `Groups the server lists for ${account.username} appear in the sidebar.`
                    : 'This account is not signed in on this Mac.'}
                </small>
              </InsetRow>
            );
          })
        ) : (
          <InsetRow label="None">No accounts configured on this Mac.</InsetRow>
        )}
      </Inset>
      {attention.length ? (
        <>
          <SectionLabel>Needs attention</SectionLabel>
          <Inset className="settings-inset middle">
            {attention.map((store) => {
              const state = storeDescriptionState(world, store);
              const server = serverOf(world, store.id);
              const caption = storeDescription(world, store);
              return (
                <InsetRow
                  key={store.id}
                  valueClass="group-attention"
                  action={
                    <Button size="sm" onClick={() => onOpen(store)}>
                      Open
                    </Button>
                  }
                >
                  <span className="who2">
                    <Stack parties={partiesOf(world, store.id)} size="lg" />
                    <span className="t">
                      <b>{store.name}</b>
                      <small>
                        {caption} · {server?.name ?? store.server}
                      </small>
                    </span>
                  </span>
                  <Chip tone="warn">
                    {state === 'inactive' ? 'Inactive' : 'Unavailable'}
                  </Chip>
                </InsetRow>
              );
            })}
          </Inset>
        </>
      ) : null}
      <SectionLabel>Invite someone</SectionLabel>
      <Inset className="settings-inset">
        {accounts.length ? (
          accounts.map((store) => {
            const account = world.accounts.find(
              (candidate) => candidate.store === store.id,
            );
            const server = serverOf(world, store.id);
            return (
              <InsetRow
                key={store.id}
                label={server?.name ?? store.server}
                action={
                  <Button
                    size="sm"
                    disabled={!account}
                    onClick={() => onInvite(store)}
                  >
                    {account
                      ? `Invite as ${account.username}…`
                      : 'Invite someone…'}
                  </Button>
                }
              />
            );
          })
        ) : (
          <InsetRow label="None">No accounts configured on this Mac.</InsetRow>
        )}
      </Inset>
    </>
  );
}

export function SettingsScreen({
  world,
  bridge,
  location,
  scene,
  onNavigate,
  onRefresh,
  onRefreshWorld,
  onError,
  onMutationError,
  onLock,
}: Props): ReactNode {
  // Capture initial fixture scene once; the shell canonicalizes the route to 'settings' on mount.
  const [enteredScene] = useState(scene);
  const section =
    location.section === 'phrase' ? 'macs' : (location.section ?? 'macs');
  const stores = accountStores(world);
  // Select account by exact StoreRef to avoid ambiguous profile-local aliases.
  const requested = location.store;
  const selected = requested
    ? stores.find((store) => store.id === requested)
    : stores[0];
  const unavailable = requested !== undefined && selected === undefined;
  const [sheet, setSheet] = useState<Sheet>(() =>
    enteredScene === 'settings-phrase'
      ? 'phrase'
      : enteredScene === 'settings-enrol'
        ? 'enrol'
        : null,
  );
  const [devices, setDevices] = useState<AccountDevice[]>([]);
  const [backups, setBackups] = useState<BackupEnrollment[]>([]);
  const [yubi, setYubi] = useState<YubiEnrollment[]>([]);
  const [cards, setCards] = useState<{ serial: number }[]>([]);
  const [pendingYubi, setPendingYubi] = useState<SimpleYubiAction | null>(null);
  const [passphraseMode, setPassphraseMode] = useState<
    'set' | 'change' | 'verify'
  >('set');
  const [passphraseStore, setPassphraseStore] = useState<AccountStore | null>(
    null,
  );
  const [pairMode, setPairMode] = useState<'offer' | 'accept'>('offer');
  const [removeDevice, setRemoveDevice] = useState<AccountDevice | null>(null);
  const [revokeBackup, setRevokeBackup] = useState<BackupEnrollment | null>(
    null,
  );
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [statuses, setStatuses] = useState<Map<string, ServerStatusSnapshot>>(
    new Map(),
  );
  const [loadedProfiles, setLoadedProfiles] = useState<Set<string>>(new Set());
  const [accountDeviceNames, setAccountDeviceNames] = useState<
    Map<string, string>
  >(new Map());
  const [macsLoaded, setMacsLoaded] = useState(false);
  const [keysLoaded, setKeysLoaded] = useState(false);
  const [groupCreate, setGroupCreate] = useState(enteredScene === 'create');
  const [inviteStore, setInviteStore] = useState<StoreRef | null>(() =>
    enteredScene === 'join-invite'
      ? (stores.find((store) => store.id === 'acct:work')?.id ??
        stores[0]?.id ??
        null)
      : null,
  );
  const [discovering, setDiscovering] = useState<StoreRef | null>(null);
  const toasts = useToast();
  // Track open modal state to suppress background catalog reloads while sheets are active.
  const sheetOpen = useRef(sheet);
  sheetOpen.current = sheet;
  const catalogRecovery = useRef<Promise<World> | null>(null);
  const recoveredAccounts = useRef(new Set<string>());
  // Deduplicate concurrent catalog recovery requests.
  const recoverCatalog = useCallback((): Promise<World> => {
    if (!catalogRecovery.current) {
      const pending = onRefreshWorld().finally(() => {
        if (catalogRecovery.current === pending) catalogRecovery.current = null;
      });
      catalogRecovery.current = pending;
    }
    return catalogRecovery.current;
  }, [onRefreshWorld]);
  // Profile of the currently selected account, or empty if none selected.
  const profile = selected?.server ?? '';

  // Ask one account's server which groups it belongs to. Discovery writes
  // durable local bindings, so it runs through the profile work queue and any
  // failure is reconciled like a mutation rather than replayed blindly.
  const discover = useCallback(
    async (context: DiscoveryContext): Promise<void> => {
      setDiscovering(context.store.id);
      try {
        const result = await enqueueProfileWork(bridge, context.server.id, () =>
          bridge.discoverGroups(context.server.id, context.account.alias),
        );
        if (result.accountAlias !== context.account.alias) {
          throw new Error(
            'Group discovery returned data for a different account.',
          );
        }
        const found = result.groups.filter((group) => group.active).length;
        await onRefresh(
          found
            ? `Found ${found} group${found === 1 ? '' : 's'} for ${context.account.username}.`
            : `No groups found for ${context.account.username}.`,
        );
      } catch (error) {
        await onMutationError(error);
      } finally {
        setDiscovering(null);
      }
    },
    [bridge, onMutationError, onRefresh],
  );

  useEffect(() => {
    const conceal = (): void => {
      if (sheetOpen.current === 'go-profile') return;
      setSheet(null);
      setPendingYubi(null);
      setPassphraseStore(null);
      setRemoveDevice(null);
      setRevokeBackup(null);
      setGroupCreate(false);
      setInviteStore(null);
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

  // Canonicalize default settings route to the first account's exact StoreRef.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({
      kind: 'settings',
      ...(location.section ? { section: location.section } : {}),
      store: selected.id,
      ...(location.profile ? { profile: location.profile } : {}),
    });
  }, [
    location.profile,
    location.section,
    location.store,
    onNavigate,
    selected,
  ]);

  // Reset active sheets and account-specific state when switching accounts.
  const selectedId = selected?.id;
  const shown = useRef(selectedId);
  useEffect(() => {
    if (shown.current === selectedId) return;
    shown.current = selectedId;
    setSheet(null);
    setPendingYubi(null);
    setPassphraseStore(null);
    setRemoveDevice(null);
    setRevokeBackup(null);
    setDevices([]);
    setBackups([]);
    setYubi([]);
    setCards([]);
    setMacsLoaded(false);
    setKeysLoaded(false);
    recoveredAccounts.current = new Set();
  }, [selectedId]);

  useEffect(() => {
    let alive = true;
    const profiles = [
      ...new Set(accountStores(world).map((store) => store.server)),
    ];
    void (async () => {
      const next = new Map<string, ServerStatusSnapshot>();
      const loaded = new Set<string>();
      for (const candidate of profiles) {
        const server = world.servers.find((item) => item.id === candidate);
        if (server?.state === 'blocked') {
          loaded.add(candidate);
          continue;
        }
        try {
          const status = await sharedServerStatus(bridge, candidate);
          if (status.profile !== candidate)
            throw new Error(
              'describe_server_status returned a different profile.',
            );
          next.set(candidate, status);
          loaded.add(candidate);
        } catch (error) {
          // Mark as loaded so UI status gates resolve even if the server probe fails.
          loaded.add(candidate);
          if (alive && shouldReportPassiveServerStatusError(error))
            onError(error);
        }
      }
      if (alive) {
        setStatuses(next);
        setLoadedProfiles(loaded);
      }
    })();
    return () => {
      alive = false;
    };
  }, [bridge, onError, world]);

  const accessStopped = (candidate: string): boolean => {
    const server = world.servers.find((item) => item.id === candidate);
    if (
      !bridge.native &&
      bridge.fixtureWorld &&
      enteredScene.startsWith('settings-') &&
      enteredScene !== 'settings-account'
    ) {
      return server?.state === 'blocked';
    }
    if (
      server?.state === 'blocked' ||
      server?.state === 'lease-lapsed' ||
      server?.state === 'never-probed'
    )
      return true;
    if (!loadedProfiles.has(candidate)) return false;
    const status = statuses.get(candidate);
    return !status?.host || serverLeaseState(status) !== 'fresh';
  };
  const selectedStopped = selected
    ? world.unavailableStores.includes(selected.id) ||
      accessStopped(selected.server)
    : true;
  const selectedStatusLoaded = selected
    ? loadedProfiles.has(selected.server)
    : false;
  const statusPending = Boolean(
    selected && !unavailable && !selectedStatusLoaded,
  );

  useEffect(() => {
    let alive = true;
    if (!selectedStatusLoaded) return;
    if (!selected || selectedStopped) {
      setDevices([]);
      setBackups([]);
      setMacsLoaded(true);
      return;
    }
    // Capture target store before await to avoid applying stale device responses.
    const requestedStore = selected.id;
    const requestedProfile = selected.server;
    setMacsLoaded(false);
    const load = (): Promise<{
      nextDevices: AccountDevice[];
      nextBackups: BackupEnrollment[];
    }> =>
      enqueueProfileWork(bridge, requestedProfile, async () => {
        const nextDevices = await bridge.listAccountDevices(requestedStore);
        const nextBackups = await bridge.listBackupEnrollments(requestedStore);
        return { nextDevices, nextBackups };
      });
    void (async () => {
      try {
        let result: {
          nextDevices: AccountDevice[];
          nextBackups: BackupEnrollment[];
        };
        try {
          result = await load();
        } catch (error) {
          // If catalog was invalidated, await world recovery and retry the query once.
          if (!alive) return;
          if (sheetOpen.current || !isCatalogRequired(error)) throw error;
          const already = recoveredAccounts.current.has(requestedStore);
          if (already && !catalogRecovery.current) throw error;
          recoveredAccounts.current.add(requestedStore);
          await recoverCatalog();
          if (!alive) return;
          result = await load();
        }
        if (alive) {
          recoveredAccounts.current.delete(requestedStore);
          setDevices(result.nextDevices);
          setBackups(result.nextBackups);
          setMacsLoaded(true);
        }
      } catch (error) {
        if (alive) {
          onError(error);
          setMacsLoaded(true);
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [
    bridge,
    onError,
    recoverCatalog,
    selected,
    selectedStatusLoaded,
    selectedStopped,
  ]);

  useEffect(() => {
    let alive = true;
    if (!selectedStatusLoaded) return;
    if (!profile || selectedStopped) {
      setCards([]);
      setYubi([]);
      setKeysLoaded(true);
      return;
    }
    // Discard responses if selected profile changed while query was in-flight.
    setKeysLoaded(false);
    void enqueueProfileWork(bridge, profile, async () => {
      const nextCards = await bridge.listYubiCards(profile);
      const nextYubi = await bridge.listYubiAccounts(profile);
      return { nextCards, nextYubi };
    })
      .then(({ nextCards, nextYubi }) => {
        if (alive) {
          setCards(nextCards);
          setYubi(nextYubi);
          setKeysLoaded(true);
        }
      })
      .catch(onError);
    return () => {
      alive = false;
    };
  }, [bridge, onError, profile, selectedStatusLoaded, selectedStopped]);

  useEffect(() => {
    let alive = true;
    if (section !== 'account') {
      setAccountDeviceNames(new Map());
      return;
    }
    void (async () => {
      try {
        const next = new Map<string, string>();
        for (const store of accountStores(world)) {
          const server = world.servers.find(
            (entry) => entry.id === store.server,
          );
          const status = statuses.get(store.server);
          const fixtureStopped = Boolean(
            bridge.fixtureWorld &&
            enteredScene === 'settings-account' &&
            store.id === 'acct:work',
          );
          const pending = !loadedProfiles.has(store.server);
          const stopped =
            world.unavailableStores.includes(store.id) ||
            fixtureStopped ||
            server?.state === 'blocked' ||
            server?.state === 'lease-lapsed' ||
            server?.state === 'never-probed' ||
            pending ||
            !status?.host ||
            serverLeaseState(status) !== 'fresh';
          if (stopped) continue;
          const loadCurrent = (): Promise<AccountDevice | undefined> =>
            enqueueProfileWork(bridge, store.server, () =>
              bridge.listAccountDevices(store.id),
            ).then((devices) => devices.find((device) => device.current));
          let current: AccountDevice | undefined;
          try {
            current = await loadCurrent();
          } catch (error) {
            if (!alive) return;
            if (sheetOpen.current || !isCatalogRequired(error)) throw error;
            const already = recoveredAccounts.current.has(store.id);
            if (already && !catalogRecovery.current) throw error;
            recoveredAccounts.current.add(store.id);
            await recoverCatalog();
            if (!alive) return;
            current = await loadCurrent();
          }
          if (current) {
            recoveredAccounts.current.delete(store.id);
            next.set(
              store.id,
              current.name ??
                (current.id.startsWith('08') ? 'Security key' : 'Device'),
            );
          }
        }
        if (alive) setAccountDeviceNames(next);
      } catch (error: unknown) {
        if (alive) onError(error);
      }
    })();
    return () => {
      alive = false;
    };
  }, [
    bridge,
    enteredScene,
    loadedProfiles,
    onError,
    recoverCatalog,
    section,
    statuses,
    world,
  ]);

  useEffect(() => {
    if (selectedStatusLoaded && selectedStopped) setSheet(null);
  }, [selectedStatusLoaded, selectedStopped]);

  useEffect(() => {
    let alive = true;
    void bridge
      .appInfo()
      .then((info) => {
        if (alive) setAppInfo(info);
      })
      .catch(onError);
    return () => {
      alive = false;
    };
  }, [bridge, onError]);

  // Retain selected account when navigating between settings sections.
  const go = (next: SettingsSection, store = selected?.id): void =>
    onNavigate({
      kind: 'settings',
      section: next,
      ...(store ? { store } : {}),
    });
  const applied = async (message: string): Promise<void> => {
    setSheet(null);
    await onRefresh(message);
    toasts.show(message);
  };
  const account = selected
    ? world.accounts.find((entry) => entry.store === selected.id)
    : undefined;
  const server = selected
    ? world.servers.find((entry) => entry.id === selected.server)
    : undefined;
  const macsLoading = Boolean(
    selected &&
    !unavailable &&
    !selectedStopped &&
    (statusPending || !macsLoaded),
  );
  const keysLoading = Boolean(
    selected &&
    !unavailable &&
    !selectedStopped &&
    (statusPending || !keysLoaded),
  );
  const createContext = stores[0];
  const invitedStore = stores.find((store) => store.id === inviteStore);
  const invitedAccount = invitedStore
    ? world.accounts.find((entry) => entry.store === invitedStore.id)
    : undefined;
  const invitedServer = invitedStore
    ? serverOf(world, invitedStore.id)
    : undefined;
  const inviteMessage =
    invitedStore && invitedAccount
      ? `1. Install FOKS: https://foks.app/download\n2. When prompted for a server address, enter ${invitedServer?.name ?? invitedStore.server}\n3. Create your account with username firstname.lastname\n4. Send your username to ${invitedAccount.username}. The account administrator can then add you to the group.`
      : '';
  return (
    <>
      <PageHeader
        title="Settings"
        subtitle="Manage accounts, servers, security keys, and local devices"
      />
      <div className="body">
        <div className="settings-cols">
          <nav className="settings-sections" aria-label="Settings sections">
            {SECTIONS.map((item) => (
              <button
                type="button"
                key={item.id}
                className={item.id === section ? 'nav on' : 'nav'}
                onClick={() => go(item.id)}
              >
                <Icon name={item.icon} />
                <span className="t">{item.label}</span>
              </button>
            ))}
          </nav>
          <div className="settings-main">
            {unavailable ? (
              <UnavailableAccount
                stores={stores}
                world={world}
                onSelect={(store) => go(section, store.id)}
                onRefresh={() => void onRefresh('Accounts refreshed')}
              />
            ) : null}
            {section === 'macs' && !unavailable ? (
              <MacsSection
                stores={stores}
                selected={selected}
                devices={devices}
                backups={backups}
                world={world}
                stopped={selectedStopped}
                loading={macsLoading}
                onSwitch={(store) =>
                  onNavigate({
                    kind: 'settings',
                    section: 'macs',
                    store: store.id,
                  })
                }
                onSheet={setSheet}
                onPair={(mode) => {
                  setPairMode(mode);
                  setSheet('pair');
                }}
                onRemove={(device) => {
                  setRemoveDevice(device);
                  setSheet('remove-device');
                }}
                onRevokeBackup={(backup) => {
                  setRevokeBackup(backup);
                  setSheet('revoke-backup');
                }}
              />
            ) : null}
            {section === 'keys' && !unavailable ? (
              <KeysSection
                yubi={yubi}
                cards={cards}
                stopped={selectedStopped}
                loading={keysLoading}
                onSheet={setSheet}
                onAction={(command) => {
                  setSheet('yubi');
                  setPendingYubi(command);
                }}
              />
            ) : null}
            {section === 'account' ? (
              <>
                <AccountSection
                  world={world}
                  statuses={statuses}
                  loadedProfiles={loadedProfiles}
                  deviceNames={accountDeviceNames}
                  onConnectGoProfile={() => setSheet('go-profile')}
                  onPassphrase={(store, mode) => {
                    setPassphraseStore(store);
                    setPassphraseMode(mode);
                    setSheet('passphrase');
                  }}
                  fixtureLapsed={Boolean(
                    bridge.fixtureWorld && enteredScene === 'settings-account',
                  )}
                />
                {accountStores(world).map((store) => (
                  <BotPanel
                    key={`bot-${store.id}`}
                    bridge={bridge}
                    profile={store.server}
                    account={store.account}
                    onComplete={() => onRefresh('Bot account updated')}
                  />
                ))}
                {accountStores(world).map((store) => (
                  <RenamePanel
                    key={`rename-${store.id}`}
                    bridge={bridge}
                    profile={store.server}
                    account={store.account}
                    onComplete={() => onRefresh('Username updated')}
                  />
                ))}
                {accountStores(world).map((store) => (
                  <SsoPanel
                    key={store.id}
                    bridge={bridge}
                    profile={store.server}
                    account={store.account}
                    login={true}
                    onComplete={() =>
                      onRefresh('Organization sign-in verified')
                    }
                  />
                ))}
              </>
            ) : null}
            {section === 'servers' ? (
              <ServersSection
                world={world}
                bridge={bridge}
                profile={location.profile}
                scene={enteredScene}
                onNavigate={onNavigate}
                onRefresh={onRefresh}
                onError={onError}
                onMutationError={onMutationError}
              />
            ) : null}
            {section === 'groups' ? (
              <GroupsSection
                world={world}
                discovering={discovering}
                onDiscover={discover}
                onCreate={() => setGroupCreate(true)}
                onInvite={(store) => setInviteStore(store.id)}
                onOpen={(store) => onNavigate({ kind: 'store', ref: store.id })}
              />
            ) : null}
            {section === 'about' ? (
              <AboutSection
                world={world}
                bridge={bridge}
                appInfo={appInfo}
                onRefresh={onRefresh}
                onError={onError}
                onMessage={(text: string) => toasts.show(text)}
                onLock={onLock}
              />
            ) : null}
          </div>
        </div>
      </div>
      {sheet === 'go-profile' ? (
        <GoProfileConnectSheet
          bridge={bridge}
          onClose={() => setSheet(null)}
          onConnected={async (_profile, alias) => {
            setSheet(null);
            await onRefreshWorld();
            toasts.show(`Connected account "${alias}" from FOKS CLI`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'phrase' && selected && account && server ? (
        <PhraseSheet
          bridge={bridge}
          profile={selected.server}
          accountAlias={selected.account}
          username={account.username}
          server={server.name}
          seedPhrase={
            enteredScene === 'settings-phrase'
              ? bridge.firstRunFixture?.backupPhrase
              : undefined
          }
          onClose={() => setSheet(null)}
          onDone={async () => applied('Backup phrase enrolled successfully.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'pair' && selected ? (
        <PairSheet
          bridge={bridge}
          store={selected}
          initialMode={pairMode}
          onClose={() => setSheet(null)}
          onDone={async (message) => applied(message)}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'recover' && selected ? (
        <RecoverSheet
          bridge={bridge}
          store={selected}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Recovery submitted successfully.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'enrol' && selected ? (
        <EnrollSheet
          bridge={bridge}
          store={selected}
          card={cards[0]}
          onClose={() => setSheet(null)}
          onDone={async () => applied('YubiKey account created successfully.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'provision' && selected ? (
        <ProvisionSheet
          bridge={bridge}
          store={selected}
          cards={cards}
          onClose={() => setSheet(null)}
          onDone={async () => applied('YubiKey added to account successfully')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'yubi' && selected && pendingYubi ? (
        <YubiActionSheet
          bridge={bridge}
          store={selected}
          action={pendingYubi}
          alias={
            (pendingYubi === 'resume-enrollment'
              ? yubi.find((entry) => entry.state === 'pending')
              : yubi.find((entry) => entry.state === 'complete')
            )?.alias ?? ''
          }
          onClose={() => {
            setSheet(null);
            setPendingYubi(null);
          }}
          onDone={async () => {
            setPendingYubi(null);
            await applied('Security key updated.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'revoke' &&
      selected &&
      yubi.some((entry) => entry.state === 'complete') ? (
        <RevokeSheet
          bridge={bridge}
          store={selected}
          alias={yubi.find((entry) => entry.state === 'complete')?.alias ?? ''}
          onClose={() => setSheet(null)}
          onDone={async () => applied('YubiKey revoked successfully')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'passphrase' && passphraseStore ? (
        <PassphraseSheet
          bridge={bridge}
          store={passphraseStore}
          initialMode={passphraseMode}
          onClose={() => {
            setSheet(null);
            setPassphraseStore(null);
          }}
          onDone={(message) => {
            setSheet(null);
            setPassphraseStore(null);
            if (passphraseMode === 'verify') toasts.show(message);
            else
              void onRefresh(message).catch((error) => onMutationError(error));
          }}
          onError={(error) => {
            if (passphraseMode === 'verify') onError(error);
            else void onMutationError(error);
          }}
        />
      ) : null}
      {sheet === 'remove-device' && selected && removeDevice ? (
        <RemoveDeviceSheet
          bridge={bridge}
          store={selected}
          device={removeDevice}
          onClose={() => {
            setSheet(null);
            setRemoveDevice(null);
          }}
          onDone={async () => {
            const name = removeDevice.name ?? 'device';
            setRemoveDevice(null);
            await applied(`Device "${name}" removed`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'revoke-backup' && selected && revokeBackup ? (
        <RevokeBackupSheet
          bridge={bridge}
          store={selected}
          backup={revokeBackup}
          onClose={() => {
            setSheet(null);
            setRevokeBackup(null);
          }}
          onDone={async () => {
            const alias = revokeBackup.backupAlias;
            setRevokeBackup(null);
            await applied(`Revoked backup phrase ${alias}`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {groupCreate && createContext ? (
        <GroupSheet
          world={world}
          bridge={bridge}
          store={createContext}
          sheet="create"
          target={null}
          onClose={() => setGroupCreate(false)}
          onSwitch={() => undefined}
          onApplied={async (message, created) => {
            const next = await onRefreshWorld();
            toasts.show(message);
            if (!created) return;
            const accountStore = next.stores.find(
              (candidate) =>
                candidate.id === created.accountStoreId &&
                candidate.kind === 'account',
            );
            const createdStore = accountStore
              ? next.stores.find(
                  (candidate): candidate is TeamStore =>
                    candidate.kind === 'team' &&
                    candidate.server === accountStore.server &&
                    candidate.account === accountStore.account &&
                    candidate.alias === created.teamAlias,
                )
              : undefined;
            if (createdStore)
              onNavigate({ kind: 'store', ref: createdStore.id });
          }}
          onError={onError}
          onMutationError={onMutationError}
        />
      ) : null}
      {inviteStore ? (
        <SheetDialog
          onClose={() => setInviteStore(null)}
          glyph={<span className="kico md invite">I</span>}
          title={
            invitedStore && invitedAccount
              ? `Invite to ${invitedServer?.name ?? invitedStore.server}`
              : 'Account no longer available'
          }
          subtitle={
            invitedStore && invitedAccount
              ? 'Help set up a new user'
              : 'Message unavailable'
          }
          footer={
            <>
              <Button onClick={() => setInviteStore(null)}>Done</Button>
              {inviteMessage ? (
                <Button
                  variant="primary"
                  onClick={() =>
                    void bridge
                      .copyText(inviteMessage)
                      .then(() => toasts.show('Message copied.'))
                      .catch(onError)
                  }
                >
                  Copy message
                </Button>
              ) : null}
            </>
          }
        >
          {invitedStore && invitedAccount ? (
            <>
              <Inset>
                <InsetRow label="Message">
                  <span className="msg">{inviteMessage}</span>
                </InsetRow>
              </Inset>
              <SectionLabel>Message contents</SectionLabel>
              <Inset className="settings-inset">
                <InsetRow label="Install link">
                  <code>https://foks.app/download</code>{' '}
                  <Chip tone="warn">placeholder</Chip>
                  <small>Temporary download address.</small>
                </InsetRow>
                <InsetRow label="Server">
                  {invitedServer?.name ?? invitedStore.server}
                  <small>
                    The invited user enters this address during setup.
                  </small>
                </InsetRow>
                <InsetRow label="Signup invite">
                  Optional
                  <small>
                    Only needed if this server requires an invitation code.
                  </small>
                </InsetRow>
                <InsetRow label="Next steps">
                  Add their username to a group in that group’s settings.
                </InsetRow>
              </Inset>
            </>
          ) : (
            <p className="fn">
              This account is no longer available. Select a different account to
              send an invite.
            </p>
          )}
        </SheetDialog>
      ) : null}
    </>
  );
}

/**
 * Displayed when the requested account cannot be found in the current catalog.
 */
function UnavailableAccount({
  stores,
  world,
  onSelect,
  onRefresh,
}: {
  stores: AccountStore[];
  world: World;
  onSelect: (store: AccountStore) => void;
  onRefresh: () => void;
}): ReactNode {
  return (
    <Notice
      severity="crit"
      title="Account no longer available"
      actions={
        <>
          <Button variant="primary" onClick={onRefresh}>
            Refresh the catalog
          </Button>
          {stores.map((store) => (
            <Button key={store.id} onClick={() => onSelect(store)}>
              {store.account} ·{' '}
              {world.servers.find((server) => server.id === store.server)
                ?.name ?? store.server}
            </Button>
          ))}
        </>
      }
    >
      <p>
        This account is no longer available on this Mac. Select another account
        below or refresh the catalog.
      </p>
      {stores.length ? null : (
        <p>
          This Mac has no active account. Add and verify a server, then create
          or recover an account.
        </p>
      )}
    </Notice>
  );
}

function MacsSection({
  stores,
  selected,
  devices,
  backups,
  world,
  stopped,
  loading,
  onSwitch,
  onSheet,
  onPair,
  onRemove,
  onRevokeBackup,
}: {
  stores: AccountStore[];
  selected?: AccountStore;
  devices: AccountDevice[];
  backups: BackupEnrollment[];
  world: World;
  stopped: boolean;
  loading: boolean;
  onSwitch: (store: AccountStore) => void;
  onSheet: (sheet: Sheet) => void;
  onPair: (mode: 'offer' | 'accept') => void;
  onRemove: (device: AccountDevice) => void;
  onRevokeBackup: (backup: BackupEnrollment) => void;
}): ReactNode {
  if (!selected)
    return (
      <Notice title="No available account on this Mac">
        <p>Add and verify a server, then create or recover an account.</p>
      </Notice>
    );
  const account = world.accounts.find((entry) => entry.store === selected.id);
  return (
    <>
      <div className="settings-label">
        <SegmentedControl
          label="This account"
          value={selected.id}
          onChange={(id) => {
            const next = stores.find((store) => store.id === id);
            if (next) onSwitch(next);
          }}
          items={stores.map((store) => {
            // Disambiguate duplicate account aliases across different servers.
            const ambiguous = stores.some(
              (other) =>
                other.id !== store.id && other.account === store.account,
            );
            const server =
              world.servers.find((entry) => entry.id === store.server)?.name ??
              store.server;
            return {
              id: store.id,
              label: ambiguous ? `${store.account} · ${server}` : store.account,
              title: `${store.account} on ${server}`,
            };
          })}
        />
      </div>
      <SectionLabel>This account</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow label="Signed in as">
          <b>{account?.username ?? 'Unknown user'}</b>
        </InsetRow>
        <InsetRow label="Local alias">{selected.account}</InsetRow>
      </Inset>
      {stopped ? (
        <Band severity="crit" label="Account access is stopped">
          Cannot connect to the server. Account management and security keys are
          unavailable until reconnected.
        </Band>
      ) : null}
      <SectionLabel>Your Macs</SectionLabel>
      <Inset className="settings-inset">
        {loading ? (
          <InsetRow>Loading devices…</InsetRow>
        ) : devices.length ? (
          devices.map((device) => (
            <InsetRow
              key={device.id}
              label={
                device.name ??
                (device.id.startsWith('08') ? 'YubiKey' : 'Device')
              }
              action={
                <>
                  {device.current ? (
                    <Chip tone="you">
                      {device.id.startsWith('08')
                        ? 'current security key'
                        : 'this Mac'}
                    </Chip>
                  ) : device.id.startsWith('04') ? (
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={stopped}
                      onClick={() => onRemove(device)}
                    >
                      Remove…
                    </Button>
                  ) : (
                    <Chip>Security key (managed separately)</Chip>
                  )}
                </>
              }
            >
              <b>{device.role}</b>
              <small>{device.id}</small>
            </InsetRow>
          ))
        ) : (
          <InsetRow label="None">
            {stopped
              ? 'Not listed while access is stopped'
              : 'No devices connected to this account.'}
          </InsetRow>
        )}
      </Inset>
      <SectionLabel>Pairing</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow
          label="From this Mac"
          action={
            <Button
              variant="primary"
              disabled={stopped}
              onClick={() => onPair('offer')}
            >
              Start pairing…
            </Button>
          }
        >
          Get a pairing phrase to connect another device to your account.
        </InsetRow>
        <InsetRow
          label="On this Mac"
          action={
            <Button disabled={stopped} onClick={() => onPair('accept')}>
              Accept or resume…
            </Button>
          }
        >
          Enter the pairing phrase displayed on your other FOKS device.
        </InsetRow>
      </Inset>
      <SectionLabel>Recovery</SectionLabel>
      <Inset className="settings-inset middle">
        <InsetRow
          label="Backup phrases"
          action={
            <Button disabled={stopped} onClick={() => onSheet('phrase')}>
              {backups.length ? 'Enroll another…' : 'Enroll…'}
            </Button>
          }
        >
          {loading
            ? 'Loading…'
            : backups.length
              ? `${backups.length} enrollment${backups.length === 1 ? '' : 's'} stored on this Mac`
              : 'No backup phrases stored on this Mac for this account.'}
        </InsetRow>
        {backups.map((backup) => (
          <InsetRow
            key={backup.backupId}
            label={backup.backupAlias}
            action={
              <Button
                size="sm"
                variant="danger"
                disabled={stopped}
                onClick={() => onRevokeBackup(backup)}
              >
                Revoke…
              </Button>
            }
          >
            <small>{backup.backupId}</small>
          </InsetRow>
        ))}
        <InsetRow
          label="Recover account"
          action={
            <Button disabled={stopped} onClick={() => onSheet('recover')}>
              Recover…
            </Button>
          }
        >
          Use a backup phrase to recover an existing account on this Mac.
        </InsetRow>
      </Inset>
      <p className="fn">
        Only backup phrases created on this Mac are listed here. Backup keys
        created on other devices cannot be viewed or revoked from this screen.
      </p>
    </>
  );
}

type SimpleYubiAction =
  | 'sync'
  | 'pin-status'
  | 'change-pin'
  | 'set-passphrase'
  | 'change-passphrase'
  | 'verify-passphrase'
  | 'unblock'
  | 'change-puk'
  | 'recover-management'
  | 'recover-subkey'
  | 'resume-enrollment'
  | 'resume-rotation'
  | 'rotate';

function KeysSection({
  yubi,
  cards,
  stopped,
  loading,
  onSheet,
  onAction,
}: {
  yubi: YubiEnrollment[];
  cards: { serial: number }[];
  stopped: boolean;
  loading: boolean;
  onSheet: (sheet: Sheet) => void;
  onAction: (action: SimpleYubiAction) => void;
}): ReactNode {
  const actions: readonly [SimpleYubiAction, string, string][] = [
    ['sync', 'Sync', 'Update the key account; optionally include federation.'],
    [
      'pin-status',
      'PIN status',
      'Read remaining PIN attempts from the connected card.',
    ],
    ['change-pin', 'Change PIN', 'Enter the current PIN and a new PIN.'],
    ['set-passphrase', 'Set passphrase', 'Set a new account passphrase.'],
    [
      'change-passphrase',
      'Change passphrase',
      'Enter the card PIN and a confirmed new passphrase.',
    ],
    [
      'verify-passphrase',
      'Verify passphrase',
      'Test the passphrase with the server login challenge.',
    ],
    ['unblock', 'Unblock PIN', 'Use your unlock code (PUK) to set a new PIN.'],
    ['change-puk', 'Change unlock code', 'Set a new unlock code (PUK).'],
    [
      'recover-management',
      'Restore management access',
      'Authorize this Mac to manage the security key.',
    ],
    [
      'recover-subkey',
      'Restore signing key',
      'Re-derive authentication credentials using your card PIN.',
    ],
    [
      'resume-enrollment',
      'Resume enrollment',
      'Continue an interrupted enrollment.',
    ],
    [
      'resume-rotation',
      'Resume key rotation',
      'Continue an interrupted management key rotation.',
    ],
    ['rotate', 'Rotate management key', 'Generate a new card management key.'],
  ];
  return (
    <>
      <SectionLabel>Enrolled keys</SectionLabel>
      <Inset className="settings-inset">
        {loading ? (
          <InsetRow>Loading keys…</InsetRow>
        ) : yubi.length ? (
          yubi.map((item) => (
            <InsetRow
              key={item.alias}
              label={item.alias}
              action={<Chip>{item.state}</Chip>}
            >
              Serial number unavailable
            </InsetRow>
          ))
        ) : (
          <InsetRow label="None">No YubiKeys enrolled.</InsetRow>
        )}
      </Inset>
      <SectionLabel>Connected now</SectionLabel>
      <Inset className="settings-inset">
        {loading ? (
          <InsetRow>Loading keys…</InsetRow>
        ) : cards.length ? (
          cards.map((card) => (
            <InsetRow key={card.serial} label={`YubiKey ${card.serial}`}>
              <Chip tone="ok">Connected</Chip>
            </InsetRow>
          ))
        ) : (
          <InsetRow label="None">No security key is connected.</InsetRow>
        )}
      </Inset>
      {stopped ? (
        <Band severity="crit" label="Security-key access is stopped">
          Cannot reach the server. These settings are unavailable until the
          server is reconnected.
        </Band>
      ) : null}
      <SectionLabel>Add</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow
          label="New account"
          action={
            <Button
              variant="primary"
              disabled={stopped}
              onClick={() => onSheet('enrol')}
            >
              Create on YubiKey…
            </Button>
          }
        >
          Store credentials directly on the security key from creation.
        </InsetRow>
        <InsetRow
          label="Existing account"
          action={
            <Button disabled={stopped} onClick={() => onSheet('provision')}>
              Provision…
            </Button>
          }
        >
          Add a connected security key as an authorized device for this account.
        </InsetRow>
      </Inset>
      <SectionLabel>Everyday and recovery</SectionLabel>
      <Inset className="settings-inset">
        {actions.map(([id, label, copy]) => {
          const available =
            !stopped &&
            !loading &&
            (id === 'resume-enrollment'
              ? yubi.some((entry) => entry.state === 'pending')
              : yubi.some((entry) => entry.state === 'complete'));
          return (
            <InsetRow
              key={id}
              label={label}
              action={
                <Button
                  size="sm"
                  disabled={!available}
                  title={
                    available
                      ? undefined
                      : stopped
                        ? 'Server access is stopped'
                        : loading
                          ? 'Loading account…'
                          : id === 'resume-enrollment'
                            ? 'No pending enrollment found'
                            : 'No complete enrollment found'
                  }
                  onClick={() => onAction(id)}
                >
                  {label.includes(' ') ? 'Open…' : label}
                </Button>
              }
            >
              {copy}
            </InsetRow>
          );
        })}
      </Inset>
      <SectionLabel className="danger-title">Danger</SectionLabel>
      <Inset className="danger-box settings-inset">
        <InsetRow
          label="Revoke YubiKey"
          action={
            <Button
              variant="danger"
              disabled={
                stopped || !yubi.some((entry) => entry.state === 'complete')
              }
              onClick={() => onSheet('revoke')}
            >
              Revoke…
            </Button>
          }
        >
          Rotates account keys controlled by this YubiKey.
        </InsetRow>
      </Inset>
    </>
  );
}

function AccountSection({
  world,
  statuses,
  loadedProfiles,
  deviceNames,
  onConnectGoProfile,
  onPassphrase,
  fixtureLapsed,
}: {
  world: World;
  statuses: Map<string, ServerStatusSnapshot>;
  loadedProfiles: Set<string>;
  deviceNames: Map<string, string>;
  onConnectGoProfile: () => void;
  onPassphrase: (
    store: AccountStore,
    mode: 'set' | 'change' | 'verify',
  ) => void;
  fixtureLapsed: boolean;
}): ReactNode {
  const stores = accountStores(world);
  if (!stores.length) {
    return (
      <Notice
        title="No available account on this Mac"
        actions={
          <Button variant="primary" onClick={onConnectGoProfile}>
            Connect from FOKS CLI…
          </Button>
        }
      >
        <p>
          Add and verify a server, then create or recover an account, or connect
          an existing account from the FOKS CLI.
        </p>
      </Notice>
    );
  }
  return (
    <>
      <SectionLabel
        action={
          <Button size="sm" onClick={onConnectGoProfile}>
            Connect from FOKS CLI…
          </Button>
        }
      >
        Accounts on this Mac
      </SectionLabel>
      {stores.map((store) => {
        const server = world.servers.find((entry) => entry.id === store.server);
        const account = world.accounts.find(
          (entry) => entry.store === store.id,
        );
        const status = statuses.get(store.server);
        const lease = serverLeaseState(status);
        const pending = !loadedProfiles.has(store.server);
        const lapsed =
          server?.state === 'lease-lapsed' ||
          lease === 'lapsed' ||
          (fixtureLapsed && store.id === 'acct:work');
        const inventoryUnavailable = world.unavailableStores.includes(store.id);
        const stopped =
          inventoryUnavailable ||
          lapsed ||
          server?.state === 'blocked' ||
          server?.state === 'never-probed' ||
          (!pending && (!status?.host || lease !== 'fresh'));
        const inert = stopped || pending;
        const statusLabel = pending
          ? 'Checking status…'
          : inventoryUnavailable
            ? 'Connection error'
            : lapsed
              ? 'Session expired'
              : stopped
                ? 'Status unknown'
                : status?.leaseRequired === false
                  ? 'Check-in not required'
                  : 'Connected';
        return (
          <div key={store.id}>
            <SectionLabel>
              {server?.name ?? store.server}
              {server?.label ? ` · ${server.label}` : ''}
            </SectionLabel>
            <Inset className="settings-inset">
              <InsetRow label="Username">
                <b>{account?.username ?? 'Identity unavailable'}</b>{' '}
                <Chip tone={stopped ? 'warn' : 'default'}>{statusLabel}</Chip>
              </InsetRow>
              <InsetRow label="Account alias">
                {store.account}
                <small>A local name on this Mac; not sent to the server.</small>
              </InsetRow>
              <InsetRow label="Device">
                {pending
                  ? 'Loading…'
                  : stopped
                    ? 'Not listed while access is stopped'
                    : (deviceNames.get(store.id) ?? 'Current device unknown')}
                <small>
                  This account’s current authenticated device. All devices are
                  under Recovery devices.
                </small>
              </InsetRow>
              <InsetRow label="Passphrase">
                <span className="a">
                  <Button
                    size="sm"
                    disabled={inert}
                    onClick={() => onPassphrase(store, 'set')}
                  >
                    Set…
                  </Button>
                  <Button
                    size="sm"
                    disabled={inert}
                    onClick={() => onPassphrase(store, 'change')}
                  >
                    Change…
                  </Button>
                  <Button
                    size="sm"
                    disabled={inert}
                    onClick={() => onPassphrase(store, 'verify')}
                  >
                    Verify
                  </Button>
                </span>
                <small>
                  {stopped
                    ? 'Reconnect to the server to manage your passphrase.'
                    : pending
                      ? 'Checking server connection…'
                      : 'Verify tests whether your passphrase matches the server.'}
                </small>
              </InsetRow>
            </Inset>
          </div>
        );
      })}
    </>
  );
}

function AgentSection({
  world,
  bridge,
  appInfo,
  onRefresh,
  onError,
  onMessage,
}: {
  world: World;
  bridge: Bridge;
  appInfo: AppInfo | null;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
}): ReactNode {
  const ready = world.agent.phase === 'Ready';
  return (
    <>
      <SectionLabel>Agent</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow label="Status">
          <span className={ready ? 'agent' : 'agent warn'}>
            <i />
            {world.agent.phase}
          </span>
          <small>
            The local background agent must be connected to use FOKS.
          </small>
        </InsetRow>
        <InsetRow
          label="Socket"
          valueClass="mono"
          action={
            appInfo ? (
              <Button
                size="sm"
                onClick={() =>
                  void bridge
                    .copyText(appInfo.agentSocket)
                    .then(() => onMessage('Socket path copied'))
                    .catch(onError)
                }
              >
                Copy
              </Button>
            ) : undefined
          }
        >
          {appInfo?.agentSocket ?? 'Reading app info…'}
          <small>Local connection used by this desktop app.</small>
        </InsetRow>
        <InsetRow
          label="Connection"
          action={
            ready ? undefined : (
              <Button
                variant="primary"
                onClick={() =>
                  void bridge
                    .retryAgentConnection()
                    .then(async (status) => {
                      const msg =
                        status.phase === 'Ready'
                          ? 'Connected to local agent.'
                          : `Agent connection status: ${status.phase}`;
                      await onRefresh(msg);
                      onMessage(msg);
                    })
                    .catch(onError)
                }
              >
                Retry connection
              </Button>
            )
          }
        >
          {ready
            ? 'Connected.'
            : 'Reconnect to the local agent. Incomplete operations will need to be restarted.'}
        </InsetRow>
      </Inset>
    </>
  );
}

function AboutSection({
  world,
  bridge,
  appInfo,
  onRefresh,
  onError,
  onMessage,
  onLock,
}: {
  world: World;
  bridge: Bridge;
  appInfo: AppInfo | null;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
  onLock: () => Promise<boolean>;
}): ReactNode {
  return (
    <>
      <SectionLabel>Application</SectionLabel>
      <Inset className="settings-inset middle">
        <InsetRow
          label="Lock application"
          action={
            <Button
              size="sm"
              icon="shield"
              onClick={() => {
                void onLock().then(
                  (locked) => {
                    if (!locked)
                      onMessage(
                        'Application lock is not available on this system.',
                      );
                  },
                  (error) => onError(error),
                );
              }}
            >
              Lock now
            </Button>
          }
        >
          <small>
            Require your operating-system credentials before FOKS can read vault
            data again.
          </small>
        </InsetRow>
      </Inset>
      <AgentSection
        world={world}
        bridge={bridge}
        appInfo={appInfo}
        onRefresh={onRefresh}
        onError={onError}
        onMessage={onMessage}
      />
      <SectionLabel>About</SectionLabel>
      <Inset className="settings-inset">
        <InsetRow label="Version">
          FOKS Desktop {appInfo?.version ?? '…'}
          <small>Installed application version.</small>
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
  dismissible = true,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  danger?: boolean;
  dismissible?: boolean;
}): ReactNode {
  return (
    <SheetDialog
      width="wide"
      danger={danger}
      onClose={onClose}
      dismissible={dismissible}
      title={title}
      subtitle={subtitle}
      footer={footer}
      glyph={
        <span className={`server-mark ${danger ? 'danger' : ''}`}>
          <Icon name={danger ? 'trash' : 'gear'} />
        </span>
      }
    >
      {children}
    </SheetDialog>
  );
}

function PhraseSheet({
  bridge,
  profile,
  accountAlias,
  username,
  server,
  seedPhrase,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  profile: string;
  accountAlias: string;
  username: string;
  server: string;
  seedPhrase?: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [phrase, setPhrase] = useState<string | null>(() => seedPhrase ?? null);
  const [alias, setAlias] = useState('paper-backup');
  const [written, setWritten] = useState(false);
  const [busy, setBusy] = useState(false);
  const words = phrase?.split(/\s+/) ?? [];
  return (
    <SheetFrame
      title={phrase ? 'Save backup phrase' : 'Create backup phrase'}
      subtitle={`Generate a recovery phrase for ${username} on ${server}`}
      onClose={() => {
        setPhrase(null);
        onClose();
      }}
      dismissible={!phrase}
      footer={
        <>
          {phrase ? (
            <Button
              variant="primary"
              disabled={!written || busy}
              onClick={() => {
                setBusy(true);
                const once = phrase;
                setPhrase(null);
                void bridge
                  .commitOwnerBackup(profile, accountAlias, alias, once)
                  .then(onDone)
                  .catch(onError)
                  .finally(() => setBusy(false));
              }}
            >
              Done
            </Button>
          ) : (
            <>
              <Button onClick={onClose}>Cancel</Button>
              <Button
                variant="primary"
                disabled={!alias.trim() || busy}
                onClick={() => {
                  setBusy(true);
                  void bridge
                    .prepareOwnerBackup(profile, accountAlias, alias.trim())
                    .then((result) => setPhrase(result.phrase))
                    .catch(onError)
                    .finally(() => setBusy(false));
                }}
              >
                Generate phrase
              </Button>
            </>
          )}
        </>
      }
    >
      {phrase ? (
        <>
          <p>
            This phrase is shown only once and cannot be copied. Write it down
            and keep it in a secure location.
          </p>
          <div className="words">
            {words.map((word, index) => (
              <span className="word" key={`${index}-${word}`}>
                <i>{index + 1}</i>
                {word}
              </span>
            ))}
          </div>
          <label className="checkline">
            <input
              type="checkbox"
              checked={written}
              onChange={(event) => setWritten(event.target.checked)}
            />
            I have written down these words
          </label>
        </>
      ) : (
        <Inset>
          <Field label="Backup alias" value={alias} onChange={setAlias} />
        </Inset>
      )}
    </SheetFrame>
  );
}

function PairSheet({
  bridge,
  store,
  initialMode,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  initialMode: 'offer' | 'accept';
  onClose: () => void;
  onDone: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [mode, setMode] = useState<'offer' | 'accept'>(initialMode);
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [target, setTarget] = useState(store.account);
  const [device, setDevice] = useState('This Mac');
  const [phrase, setPhrase] = useState('');
  const [busy, setBusy] = useState(false);
  const queued = <T,>(task: () => Promise<T>): Promise<T> =>
    enqueueProfileWork(bridge, store.server, task);
  const act = (task: () => Promise<unknown>, message: string): void => {
    setOffer(null);
    setPhrase('');
    setBusy(true);
    void queued(task)
      .then(() => onDone(message))
      .catch(onError)
      .finally(() => setBusy(false));
  };
  const revealOffer = (task: () => Promise<PairingOffer>): void => {
    setOffer(null);
    setBusy(true);
    void queued(task)
      .then((next) => {
        if (next.accountAlias !== store.account)
          throw new Error('pairing offer returned a different account.');
        setOffer(next);
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <SheetFrame
      title="Set up another Mac"
      subtitle="Start or resume pairing a device"
      onClose={() => {
        if (busy) return;
        setOffer(null);
        setPhrase('');
        onClose();
      }}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Close
          </Button>
          <span className="spacer" />
          {mode === 'offer' ? (
            <>
              <Button
                disabled={busy}
                onClick={() =>
                  revealOffer(() => bridge.resumeDevicePairingOffer(store.id))
                }
              >
                Resume offer
              </Button>
              <Button
                disabled={!offer || busy}
                onClick={() =>
                  act(async () => {
                    const result = await bridge.finishDevicePairing(store.id);
                    if (result.alias !== store.account)
                      throw new Error(
                        'finish_device_pairing returned a different account.',
                      );
                    return result;
                  }, 'Device paired successfully.')
                }
              >
                Finish
              </Button>
              <Button
                variant="primary"
                disabled={busy}
                onClick={() =>
                  revealOffer(() => bridge.startDevicePairing(store.id))
                }
              >
                Start
              </Button>
            </>
          ) : (
            <>
              <Button
                disabled={busy || !target}
                onClick={() =>
                  act(async () => {
                    const result = await bridge.resumeDevicePairingAcceptance(
                      store.server,
                      target,
                    );
                    if (result.alias !== target)
                      throw new Error(
                        'resume_device_pairing_acceptance returned a different account.',
                      );
                    return result;
                  }, 'Pairing acceptance resumed; refreshed authenticated devices')
                }
              >
                Resume acceptance
              </Button>
              <Button
                variant="primary"
                disabled={busy || !target || !device || !phrase}
                onClick={() =>
                  act(async () => {
                    const result = await bridge.acceptDevicePairing(
                      store.server,
                      target,
                      device,
                      phrase,
                    );
                    if (result.alias !== target)
                      throw new Error(
                        'accept_device_pairing returned a different account.',
                      );
                    return result;
                  }, 'Pairing accepted; refreshed authenticated devices')
                }
              >
                Accept
              </Button>
            </>
          )}
        </>
      }
    >
      <SegmentedControl
        label="Pairing direction"
        value={mode}
        onChange={(next) => {
          if (next === 'offer') {
            setPhrase('');
            setMode('offer');
          } else {
            setOffer(null);
            setMode('accept');
          }
        }}
        items={[
          { id: 'offer', label: 'From this Mac' },
          { id: 'accept', label: 'On this Mac' },
        ]}
      />
      {mode === 'offer' ? (
        <>
          <p>
            Select Start, enter the pairing phrase on the other Mac, then select
            Finish here. Resume opens the pending offer.
          </p>
          {offer ? (
            <Inset>
              <InsetRow label="Pairing phrase" valueClass="mono">
                {offer.phrase}
              </InsetRow>
            </Inset>
          ) : null}
        </>
      ) : (
        <>
          <p>
            Enter the pairing phrase from the other Mac. Resume continues a
            pending acceptance.
          </p>
          <Inset>
            <Field label="Account alias" value={target} onChange={setTarget} />
            <Field label="Device name" value={device} onChange={setDevice} />
            <Field
              label="Pairing phrase"
              value={phrase}
              onChange={setPhrase}
              type="password"
            />
          </Inset>
        </>
      )}
    </SheetFrame>
  );
}

function RecoverSheet({
  bridge,
  store,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [target, setTarget] = useState(store.account);
  const [device, setDevice] = useState('This Mac');
  const [phrase, setPhrase] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <SheetFrame
      title="Recover on this Mac"
      subtitle="Use your 17-word backup phrase"
      onClose={() => {
        setPhrase('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!target || !device || !phrase || busy}
            onClick={() => {
              const once = phrase;
              setPhrase('');
              setBusy(true);
              void enqueueProfileWork(bridge, store.server, () =>
                bridge.recoverOwnerAccount(store.server, target, once, device),
              )
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Recover
          </Button>
        </>
      }
    >
      <p>
        Recovery adds this Mac as an authorized device. If interrupted, you can
        resume recovery from Alerts using the same phrase.
      </p>
      <Inset>
        <Field label="Local alias" value={target} onChange={setTarget} />
        <Field label="Device name" value={device} onChange={setDevice} />
        <InsetRow label="Recovery phrase">
          <textarea
            value={phrase}
            onChange={(event) => setPhrase(event.target.value)}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}

function EnrollSheet({
  bridge,
  store,
  card,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  card?: { serial: number };
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [alias, setAlias] = useState('work-key');
  const [username, setUsername] = useState('');
  const [deviceName, setDeviceName] = useState(
    card ? `YubiKey ${card.serial}` : 'YubiKey',
  );
  const [invite, setInvite] = useState('');
  const [pin, setPin] = useState('');
  const [puk, setPuk] = useState('');
  const [signingSlot, setSigningSlot] = useState('0x82');
  const [pqSlot, setPqSlot] = useState('0x83');
  const [pinAttempts, setPinAttempts] = useState(3);
  const [pukAttempts, setPukAttempts] = useState(3);
  const [busy, setBusy] = useState(false);
  const clear = (): void => {
    setInvite('');
    setPin('');
    setPuk('');
  };
  const slot = (value: string): number | null =>
    /^0x[0-9a-fA-F]{2}$/.test(value)
      ? Number.parseInt(value.slice(2), 16)
      : null;
  const signing = slot(signingSlot);
  const pq = slot(pqSlot);
  const validAttempts =
    Number.isInteger(pinAttempts) &&
    pinAttempts > 0 &&
    pinAttempts <= 255 &&
    Number.isInteger(pukAttempts) &&
    pukAttempts > 0 &&
    pukAttempts <= 255;
  return (
    <SheetFrame
      title="Create a YubiKey account"
      subtitle={`Create a new account on ${store.server} with credentials stored directly on your hardware key`}
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={
              !card ||
              !alias.trim() ||
              !username.trim() ||
              !deviceName.trim() ||
              !pin ||
              !puk ||
              signing === null ||
              pq === null ||
              signing === pq ||
              !validAttempts ||
              busy
            }
            onClick={() => {
              if (signing === null || pq === null) return;
              const command: YubiCommand = {
                command: 'create_yubi_account',
                args: {
                  profile: store.server,
                  alias: alias.trim(),
                  username: username.trim(),
                  deviceName: deviceName.trim(),
                  email: '',
                  invite,
                  cardSerial: card?.serial ?? 0,
                  signingSlot: signing,
                  pqSlot: pq,
                  pin,
                  puk,
                  pinAttempts,
                  pukAttempts,
                },
              };
              clear();
              setBusy(true);
              void bridge
                .runYubi(command)
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Prepare card and create account
          </Button>
        </>
      }
    >
      <p>Before you continue:</p>
      <ol className="sheet-steps">
        <li>
          <b>Credentials are written in a single operation.</b> If setup fails,
          the card's security applet must be reset, erasing existing card data.
        </li>
        <li>
          <b>The card must use default factory settings.</b> Custom-managed
          cards are not supported.
        </li>
        <li>
          <b>Save your unlock code securely.</b> It cannot be recovered if lost.
        </li>
      </ol>
      {card ? (
        <p>
          <b>YubiKey {card.serial}</b> is connected. Choose an alias for this
          key below.
        </p>
      ) : (
        <Band label="Connect a YubiKey">
          No security key is currently detected.
        </Band>
      )}
      <Inset>
        <Field label="Alias" value={alias} onChange={setAlias} />
        <Field label="Username" value={username} onChange={setUsername} />
        <Field
          label="Device name"
          value={deviceName}
          onChange={setDeviceName}
        />
        <Field label="Card PIN" value={pin} onChange={setPin} type="password" />
        <Field
          label="Unlock code"
          value={puk}
          onChange={setPuk}
          type="password"
        />
        <InsetRow label="Invite">
          <input
            type="password"
            value={invite}
            onChange={(event) => setInvite(event.target.value)}
          />
          <small>Optional server invitation code.</small>
        </InsetRow>
      </Inset>
      <details className="adv">
        <summary>Advanced</summary>
        <Inset>
          <Field
            label="Signing slot"
            value={signingSlot}
            onChange={setSigningSlot}
            mono
          />
          <Field
            label="Post-quantum slot"
            value={pqSlot}
            onChange={setPqSlot}
            mono
          />
          <InsetRow label="PIN tries">
            <input
              type="number"
              min={1}
              max={255}
              value={pinAttempts}
              onChange={(event) => setPinAttempts(Number(event.target.value))}
            />
          </InsetRow>
          <InsetRow label="PUK tries">
            <input
              type="number"
              min={1}
              max={255}
              value={pukAttempts}
              onChange={(event) => setPukAttempts(Number(event.target.value))}
            />
          </InsetRow>
        </Inset>
      </details>
      <p className="hint">
        PIN, unlock code and invite go only to the local agent and are cleared
        when submitted.
      </p>
    </SheetFrame>
  );
}

function ProvisionSheet({
  bridge,
  store,
  cards,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  cards: { serial: number }[];
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [targetAlias, setTargetAlias] = useState('new-key');
  const [deviceName, setDeviceName] = useState(
    cards[0] ? `YubiKey ${cards[0].serial}` : 'YubiKey',
  );
  const [serial, setSerial] = useState(cards[0]?.serial ?? 0);
  const [pin, setPin] = useState('');
  const [puk, setPuk] = useState('');
  const [busy, setBusy] = useState(false);
  const clear = (): void => {
    setPin('');
    setPuk('');
  };
  const valid = Boolean(
    cards.some((card) => card.serial === serial) &&
    targetAlias.trim() &&
    deviceName.trim() &&
    pin &&
    puk,
  );
  return (
    <SheetFrame
      title="Provision a YubiKey device"
      subtitle={`Add a security key to ${store.account}`}
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              const command: YubiCommand = {
                command: 'provision_yubi_device',
                args: {
                  accountStoreId: store.id,
                  targetAlias: targetAlias.trim(),
                  deviceName: deviceName.trim(),
                  cardSerial: serial,
                  signingSlot: 0x82,
                  pqSlot: 0x83,
                  pin,
                  puk,
                  pinAttempts: 3,
                  pukAttempts: 3,
                },
              };
              clear();
              setBusy(true);
              void bridge
                .runYubi(command)
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Provision card
          </Button>
        </>
      }
    >
      <p>Enter a new alias and select a connected card.</p>
      {cards.length ? null : (
        <Band label="Connect a YubiKey">
          No security key is currently detected.
        </Band>
      )}
      <Inset>
        <Field
          label="Key alias"
          value={targetAlias}
          onChange={setTargetAlias}
        />
        <Field
          label="Device name"
          value={deviceName}
          onChange={setDeviceName}
        />
        {cards.length ? (
          <InsetRow label="Connected card">
            <select
              value={serial}
              onChange={(event) => setSerial(Number(event.target.value))}
            >
              {cards.map((card) => (
                <option key={card.serial} value={card.serial}>
                  YubiKey {card.serial}
                </option>
              ))}
            </select>
          </InsetRow>
        ) : null}
        <Field label="Card PIN" value={pin} onChange={setPin} type="password" />
        <Field
          label="Unlock code"
          value={puk}
          onChange={setPuk}
          type="password"
        />
      </Inset>
    </SheetFrame>
  );
}

function YubiActionSheet({
  bridge,
  store,
  action,
  alias,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  action: SimpleYubiAction;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [pin, setPin] = useState('');
  const [other, setOther] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const needsPin = !['pin-status', 'recover-management'].includes(action);
  const needsOther = [
    'change-pin',
    'set-passphrase',
    'change-passphrase',
    'verify-passphrase',
    'unblock',
    'change-puk',
  ].includes(action);
  const needsConfirmation =
    action === 'set-passphrase' || action === 'change-passphrase';
  const valid = Boolean(
    alias &&
    (!needsPin || action === 'resume-rotation' || pin) &&
    (!needsOther || other) &&
    (!needsConfirmation || other === confirmation),
  );
  const firstLabel =
    action === 'unblock' || action === 'change-puk'
      ? 'Current PUK'
      : 'Card PIN';
  const otherLabel =
    action === 'change-pin' || action === 'unblock'
      ? 'New PIN'
      : action === 'change-puk'
        ? 'New PUK'
        : 'Passphrase';
  const submit = (): void => {
    let command: YubiCommand;
    switch (action) {
      case 'sync':
        command = {
          command: 'sync_yubi_account',
          args: { profile: store.server, alias, pin, withFederation: true },
        };
        break;
      case 'pin-status':
        command = {
          command: 'yubi_pin_status',
          args: { profile: store.server, alias },
        };
        break;
      case 'change-pin':
        command = {
          command: 'change_yubi_pin',
          args: { profile: store.server, alias, oldPin: pin, newPin: other },
        };
        break;
      case 'set-passphrase':
      case 'change-passphrase':
        command = {
          command:
            action === 'set-passphrase'
              ? 'set_yubi_passphrase'
              : 'change_yubi_passphrase',
          args: {
            profile: store.server,
            alias,
            pin,
            passphrase: other,
            confirmation,
          },
        };
        break;
      case 'verify-passphrase':
        command = {
          command: 'verify_yubi_passphrase',
          args: { profile: store.server, alias, pin, passphrase: other },
        };
        break;
      case 'unblock':
        command = {
          command: 'unblock_yubi_pin',
          args: { profile: store.server, alias, puk: pin, newPin: other },
        };
        break;
      case 'change-puk':
        command = {
          command: 'change_yubi_puk',
          args: { profile: store.server, alias, oldPuk: pin, newPuk: other },
        };
        break;
      case 'recover-management':
        command = {
          command: 'recover_yubi_management_key',
          args: { accountStoreId: store.id, yubiAlias: alias },
        };
        break;
      case 'recover-subkey':
        command = {
          command: 'recover_yubi_subkey',
          args: { profile: store.server, alias, pin },
        };
        break;
      case 'resume-enrollment':
        command = {
          command: 'resume_yubi_account',
          args: { profile: store.server, alias, pin },
        };
        break;
      case 'resume-rotation':
        command = {
          command: 'resume_yubi_management_key',
          args: { profile: store.server, alias, ...(pin ? { pin } : {}) },
        };
        break;
      case 'rotate':
        command = {
          command: 'rotate_yubi_management_key',
          args: { profile: store.server, alias, pin },
        };
        break;
    }
    setPin('');
    setOther('');
    setConfirmation('');
    setBusy(true);
    void bridge
      .runYubi(command)
      .then(onDone)
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <SheetFrame
      title={action
        .split('-')
        .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
        .join(' ')}
      subtitle={alias || 'Choose an enrolled key alias'}
      onClose={() => {
        setPin('');
        setOther('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!valid || busy} onClick={submit}>
            Continue
          </Button>
        </>
      }
    >
      <p>
        The values below go only to the local agent and are cleared when
        submitted.
      </p>
      <Inset>
        <InsetRow label="Key alias">
          <b>{alias || 'No key enrolled'}</b>
        </InsetRow>
        {needsPin ? (
          <InsetRow label={firstLabel}>
            <input
              type="password"
              value={pin}
              onChange={(event) => setPin(event.target.value)}
            />
          </InsetRow>
        ) : null}
        {needsOther ? (
          <InsetRow label={otherLabel}>
            <input
              type="password"
              value={other}
              onChange={(event) => setOther(event.target.value)}
            />
          </InsetRow>
        ) : null}
        {needsConfirmation ? (
          <Field
            label="Confirm"
            value={confirmation}
            onChange={setConfirmation}
            type="password"
          />
        ) : null}
      </Inset>
    </SheetFrame>
  );
}

function RevokeSheet({
  bridge,
  store,
  alias,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <SheetFrame
      title={`Revoke ${alias}?`}
      subtitle="Disconnects this key and updates account security"
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      danger
      dismissible={!busy}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={confirmation !== alias || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .runYubi({
                  command: 'revoke_yubi_device',
                  args: {
                    accountStoreId: store.id,
                    yubiAlias: alias,
                    confirmation,
                  },
                })
                .then((result) => {
                  if (
                    result.alias !== alias ||
                    result.removedLocalCredential !== true
                  )
                    throw new Error(
                      'revoke_yubi_device returned a different enrollment.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Revoke {alias}
          </Button>
        </>
      }
    >
      <p>
        This YubiKey will immediately lose access to your account. Any data
        previously cached on devices using this key will remain until cleared.
        Enter the key alias to confirm revocation.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${alias}`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}

function RevokeBackupSheet({
  bridge,
  store,
  backup,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  backup: BackupEnrollment;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <SheetFrame
      title={`Revoke ${backup.backupAlias}?`}
      subtitle={`${store.account} · ${backup.backupId}`}
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      danger
      dismissible={!busy}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={confirmation !== backup.backupAlias || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .revokeOwnerBackup(store.id, backup, confirmation)
                .then((revoked) => {
                  if (
                    revoked.backupAlias !== backup.backupAlias ||
                    revoked.backupId !== backup.backupId
                  )
                    throw new Error(
                      'revoke_owner_backup returned a different enrollment.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Revoke backup phrase
          </Button>
        </>
      }
    >
      <p>
        This phrase will immediately lose future recovery access. Revocation
        also rotates every account key the backup could read. Enter the backup
        alias to confirm.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${backup.backupAlias}`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}

function RemoveDeviceSheet({
  bridge,
  store,
  device,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  device: AccountDevice;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const expected = device.name ?? device.id;
  return (
    <SheetFrame
      title={`Remove ${device.name ?? 'device'}?`}
      subtitle={`${store.account} · ${device.id}`}
      onClose={onClose}
      danger
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={confirmation !== expected || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .removeAccountDevice(store.id, device.id)
                .then((removed) => {
                  if (removed.deviceId !== device.id)
                    throw new Error(
                      'remove_account_device returned a different device.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Remove device
          </Button>
        </>
      }
    >
      <p>
        This device loses future access to the account. Any data previously
        downloaded to this device will remain until removed; change any
        sensitive secrets if the device is not under your control.
      </p>
      <p className="fn">
        You can only remove other devices here. YubiKeys are managed under
        Security keys.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${expected}`}
          />
        </InsetRow>
      </Inset>
    </SheetFrame>
  );
}

function PassphraseSheet({
  bridge,
  store,
  initialMode,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  initialMode: 'set' | 'change' | 'verify';
  onClose: () => void;
  onDone: (message: string) => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const [mode, setMode] = useState<'set' | 'change' | 'verify'>(initialMode);
  const [passphrase, setPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const submit = (): void => {
    const secret = passphrase;
    const repeated = confirmation;
    setBusy(true);
    const task =
      mode === 'set'
        ? bridge.setAccountPassphrase(store.id, secret, repeated)
        : mode === 'change'
          ? bridge.changeAccountPassphrase(store.id, secret, repeated)
          : bridge.verifyAccountPassphrase(store.id, secret);
    void task
      .then(() => {
        setPassphrase('');
        setConfirmation('');
        onDone(
          mode === 'verify'
            ? 'Passphrase verified successfully.'
            : 'Passphrase updated successfully.',
        );
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <SheetFrame
      title="Account passphrase"
      subtitle="Set, change, or verify your account passphrase"
      onClose={() => {
        if (busy) return;
        setPassphrase('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={
              !passphrase ||
              (mode !== 'verify' && passphrase !== confirmation) ||
              busy
            }
            onClick={submit}
          >
            {mode === 'verify'
              ? 'Verify'
              : mode === 'set'
                ? 'Set passphrase'
                : 'Change passphrase'}
          </Button>
        </>
      }
    >
      <SegmentedControl
        label="Passphrase action"
        value={mode}
        onChange={setMode}
        items={[
          { id: 'set', label: 'Set' },
          { id: 'change', label: 'Change' },
          { id: 'verify', label: 'Verify' },
        ]}
      />
      <Inset>
        <Field
          label="Passphrase"
          value={passphrase}
          onChange={setPassphrase}
          type="password"
        />
        {mode === 'verify' ? null : (
          <Field
            label="Confirm"
            value={confirmation}
            onChange={setConfirmation}
            type="password"
          />
        )}
      </Inset>
    </SheetFrame>
  );
}
