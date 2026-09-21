import type { RefreshOperation } from '../refresh-activity';
import {
  catalogItemsComplete,
  catalogStoreComplete,
  projectCatalogFreshness,
  sameStoreIdentity,
} from '../catalog-state';
import { itemKey, serverDisplayName, serverFactAvailability } from '../model';
import { consumeProfileRosterStaleness } from '../roster-staleness';
import type {
  AgentSnapshot,
  AgentStatus,
  FederationEntry,
  GroupDetailFailure,
  Item,
  Party,
  Server,
  ServerRestriction,
  Store,
  StoreRef,
} from '../model';
import { retainProfileConnection } from '../profile-connectivity';
import {
  scheduleProfileWork,
  type BackgroundHistoryWork,
} from '../scheduling/profile-work';
import type { Bridge } from './contract';
import { diagnosticLog, outcomeForCode } from '../diagnostics/log';
import {
  isHardStateSchemaFailure,
  normalizeCommandError,
  reportReadinessError,
} from './errors';
import { sharedServerStatus } from './profile-work';
import {
  notificationsOf,
  restrictionFromError,
  schemaInstruction,
} from './snapshot-notifications';
import type { CatalogDto } from './vault-catalog';

function recoverableGroupDetailFailure(
  error: unknown,
  store: StoreRef,
  source: GroupDetailFailure['source'],
): GroupDetailFailure {
  const typed = normalizeCommandError(error);
  if (
    ![
      'rate-limited',
      'quota-exceeded',
      'capability-denied',
      'operation-failed',
      'io',
      'deadline-exceeded',
      'profile-busy',
      'busy',
      'cancelled',
      // A part the native side could not decode: the team fails closed on
      // its own record, like one the agent could not read.
      'invalid-response',
    ].includes(typed.code)
  ) {
    if (typed.code === 'invalid-command-error') {
      throw new Error('The agent returned an unrecognized group-detail error.');
    }
    throw typed;
  }
  return {
    store,
    source,
    code: typed.code,
    message: typed.message,
    retryable: typed.retryable,
    ...(typed.details
      ? {
          details: Object.fromEntries(
            Object.entries(typed.details).filter(
              ([, value]) => value !== undefined,
            ),
          ),
        }
      : {}),
  };
}

export async function projectCatalog(
  bridge: Bridge,
  response: CatalogDto,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  agent: AgentStatus,
  partial: boolean,
  profileScope?: string,
  background?: BackgroundHistoryWork,
  /**
   * Read every team's roster even when its chain has not moved, for a
   * refresh the user asked for.
   */
  forceRosters = false,
  activity?: RefreshOperation,
): Promise<AgentSnapshot> {
  const operation = activity?.child('Loading catalog details');
  // The projection is the renderer's own share of a catalog read: the server
  // and account listings, and a roster read per team whose chain moved. Its
  // duration and those counts are what the timing log keeps of it.
  const end = diagnosticLog.span('catalog.project', {
    scope: profileScope,
    attrs: { partial, forced: forceRosters },
  });
  const counts = { servers: 0, rosters: 0 };
  try {
    const snapshot = await projectCatalogCounted(
      bridge,
      response,
      base,
      nowSeconds,
      agent,
      partial,
      profileScope,
      background,
      forceRosters,
      counts,
      operation,
    );
    end('ok', { attrs: counts });
    return snapshot;
  } catch (error) {
    const code = normalizeCommandError(error).code;
    end(outcomeForCode(code), { code, attrs: counts });
    throw error;
  } finally {
    operation?.finish();
  }
}

async function projectCatalogCounted(
  bridge: Bridge,
  response: CatalogDto,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  agent: AgentStatus,
  partial: boolean,
  profileScope: string | undefined,
  background: BackgroundHistoryWork | undefined,
  forceRosters: boolean,
  counts: { servers: number; rosters: number },
  activity?: RefreshOperation,
): Promise<AgentSnapshot> {
  // A native response that carries the locally known server facts and accounts
  // is projected from them, whichever command produced it, so the whole-catalog
  // read spends no further IPC hops on them. A bridge that supplies none of it
  // — the fixtures and the web mock — keeps the per-server reads.
  const embeddedMetadata =
    partial ||
    (bridge.native &&
      (profileScope !== undefined || response.localMetadata !== undefined));
  const globalFailure = response.failures.find((failure) =>
    ['bootstrap-required', 'agent-lost', 'version-mismatch'].includes(
      failure.error.code,
    ),
  );
  if (globalFailure) {
    reportReadinessError(globalFailure.error);
    throw globalFailure.error;
  }
  const liveStores = response.stores as Store[];
  const storesById = new Map<StoreRef, Store>();
  {
    for (const store of base?.stores ?? []) {
      const inventory = response.inventory.find(
        (entry) => entry.profile === store.server,
      );
      if (
        response.profiles.includes(store.server) &&
        !(store.kind === 'account'
          ? inventory?.accountsComplete
          : inventory?.teamsComplete)
      )
        storesById.set(store.id, store);
    }
  }
  for (const store of response.knownStores as Store[])
    storesById.set(store.id, store);
  for (const store of liveStores) storesById.set(store.id, store);
  const stores = [...storesById.values()];
  const liveStoreIds = new Set(liveStores.map((store) => store.id));
  const storeInventory = stores.map((store) => {
    const failures = response.failures.filter((entry) =>
      entry.scope === 'store'
        ? entry.store === store.id
        : entry.profile === store.server && entry.source === 'KV catalog',
    );
    const restrictions = failures.flatMap((failure) => {
      const restriction = restrictionFromError(failure.error);
      return restriction ? [restriction] : [];
    });
    const failure = failures[0];
    const complete = catalogStoreComplete(response, store, partial);
    const previous = base?.storeInventory.find(
      (entry) => entry.store === store.id,
    );
    const previousStore = base?.stores.find((entry) => entry.id === store.id);
    if (
      partial &&
      !complete &&
      !failure &&
      previous &&
      previousStore &&
      sameStoreIdentity(previousStore, store)
    )
      return previous;
    return liveStoreIds.has(store.id) && complete && !failure
      ? {
          store: store.id,
          status: 'available' as const,
          restrictions,
        }
      : {
          store: store.id,
          status:
            partial && !failure
              ? ('loading' as const)
              : ('unavailable' as const),
          restrictions,
          ...(failure ? { error: failure.error } : {}),
        };
  });
  const inventoryProfiles = new Set(
    response.inventory.map((state) => state.profile),
  );
  if (
    inventoryProfiles.size !== response.inventory.length ||
    response.inventory.some(
      (state) => !response.profiles.includes(state.profile),
    )
  ) {
    throw new Error(
      'list_catalog returned duplicate or unknown inventory profiles.',
    );
  }
  const profileInventory = response.profiles.map((profile) => {
    const inventory = response.inventory.find(
      (entry) => entry.profile === profile,
    );
    return {
      profile,
      accounts: inventory?.accountsComplete
        ? ('complete' as const)
        : ('unavailable' as const),
      teams: inventory?.teamsComplete
        ? ('complete' as const)
        : ('unavailable' as const),
    };
  });
  // When a server profile is blocked, skip roster queries for that server.
  const blockedProfiles = new Set(response.blockedProfiles);
  const loadingError = normalizeCommandError({
    code: 'catalog-loading',
    message: 'The profile catalog is still loading.',
    retryable: false,
    ambiguous: false,
    fatal: false,
  });
  const listedServers: Server[] = embeddedMetadata
    ? response.profiles.map((profile) => {
        const metadata = response.localMetadata?.profiles.find(
          (entry) => entry.profile === profile,
        );
        const previous = base?.servers.find((entry) => entry.id === profile);
        return {
          id: profile,
          name: profile,
          label: metadata?.label ?? previous?.label ?? null,
          configuredProbe:
            metadata?.configuredProbe ?? previous?.configuredProbe ?? profile,
          host_id: null,
          chain: null,
          epoch: null,
          accounts: liveStores
            .filter(
              (store) => store.server === profile && store.kind === 'account',
            )
            .map((store) => store.account),
          trust: blockedProfiles.has(profile)
            ? { status: 'blocked', error: loadingError }
            : { status: 'unknown' },
          compatibility: { status: 'requirement-unknown', error: loadingError },
          passiveStatus: {
            status: 'failed',
            source: 'describe-server-status',
            error: loadingError,
          },
          connectivity: { status: 'unknown' },
          services: { chat: null },
          restrictions: previous?.restrictions ?? [],
        };
      })
    : (await bridge.listServers(response.generation)).filter(
        (server) => profileScope === undefined || server.id === profileScope,
      );
  counts.servers = listedServers.length;
  const statusResults =
    bridge.native || partial
      ? await Promise.all(
          listedServers
            .filter(
              (server) =>
                server.trust.status !== 'blocked' &&
                !blockedProfiles.has(server.id),
            )
            .map(async (server) => {
              const statusActivity = activity?.child('Loading server status');
              try {
                const cached = response.localMetadata?.profiles.find(
                  (entry) => entry.profile === server.id,
                );
                if (embeddedMetadata && (cached?.error || !cached?.status))
                  throw (
                    cached?.error ??
                    (partial
                      ? loadingError
                      : new Error('Server status was not returned.'))
                  );
                const status = embeddedMetadata
                  ? cached!.status!
                  : await sharedServerStatus(bridge, server.id);
                if (status.profile !== server.id) {
                  throw new Error(
                    'describe_server_status returned a different profile.',
                  );
                }
                return { profile: server.id, status };
              } catch (error) {
                const typed = normalizeCommandError(error);
                if (
                  typed.code === 'bootstrap-required' ||
                  typed.code === 'agent-lost' ||
                  typed.code === 'version-mismatch'
                ) {
                  reportReadinessError(typed);
                  throw typed;
                }
                return {
                  profile: server.id,
                  error: typed,
                };
              } finally {
                statusActivity?.finish();
              }
            }),
        )
      : [];
  const statuses = new Map(
    statusResults.flatMap((result) =>
      result.status ? [[result.profile, result.status] as const] : [],
    ),
  );
  const statusFailures = new Map(
    statusResults.flatMap((result) =>
      result.error ? [[result.profile, result.error] as const] : [],
    ),
  );
  const incompatibleSchemaProfiles = new Set(
    response.failures
      .filter((failure) => isHardStateSchemaFailure(failure.error))
      .map((failure) => failure.profile),
  );
  const servers = listedServers.map((server) => {
    if (!bridge.native && !partial) return server;
    const scopedFailures = response.failures.filter(
      (failure) => failure.scope === 'profile' && failure.profile === server.id,
    );
    const restrictions: ServerRestriction[] = scopedFailures.flatMap(
      (failure) => {
        const restriction = restrictionFromError(failure.error);
        return restriction ? [restriction] : [];
      },
    );
    const status = statuses.get(server.id);
    const statusError = statusFailures.get(server.id);
    const statusRestriction = statusError && restrictionFromError(statusError);
    const previous = base?.servers.find((entry) => entry.id === server.id);
    const pendingSameIdentity =
      partial &&
      statusError?.code === 'catalog-loading' &&
      previous?.configuredProbe === server.configuredProbe;
    const allRestrictions = [
      ...(pendingSameIdentity ||
      (partial &&
        !catalogItemsComplete(response, server.id, partial) &&
        previous?.configuredProbe === server.configuredProbe)
        ? (previous?.restrictions ?? [])
        : []),
      ...restrictions,
      ...(statusRestriction ? [statusRestriction] : []),
    ];
    if (server.trust.status === 'blocked' || blockedProfiles.has(server.id)) {
      const trustFailure = scopedFailures.find((failure) =>
        [
          'rollback-detected',
          'checkpoint-reset-required',
          'server-verification-failed',
        ].includes(failure.error.code),
      )?.error;
      return {
        ...server,
        trust: {
          status: 'blocked' as const,
          error:
            trustFailure ??
            (server.trust.status === 'blocked'
              ? server.trust.error
              : normalizeCommandError({
                  code: 'server-verification-failed',
                  message: 'Server verification failed.',
                  retryable: false,
                  ambiguous: false,
                  fatal: false,
                })),
        },
        restrictions: allRestrictions,
        services: { chat: null },
      };
    }
    if (
      pendingSameIdentity &&
      previous?.trust.status === 'verified' &&
      previous.passiveStatus.status === 'available'
    )
      return {
        ...previous,
        label: server.label,
        restrictions: allRestrictions,
      };
    if (!status || statusError)
      return {
        ...server,
        trust: { status: 'unknown' as const },
        passiveStatus:
          statusError?.code === 'catalog-loading'
            ? { status: 'loading' as const }
            : {
                status: 'failed' as const,
                source: 'describe-server-status' as const,
                error:
                  statusError ??
                  normalizeCommandError(
                    new Error('Server status was not returned.'),
                  ),
              },
        compatibility: {
          status: 'requirement-unknown' as const,
          error:
            statusError ??
            normalizeCommandError(
              new Error('Compatibility status is unknown.'),
            ),
        },
        services: { chat: null },
        restrictions: allRestrictions,
      };
    return {
      ...server,
      configuredProbe: status.configuredProbe,
      host_id: status.host?.hostId ?? null,
      chain: status.host?.chain ?? null,
      epoch: status.host?.epoch ?? null,
      trust: status.host
        ? { status: 'verified' as const }
        : { status: 'unprobed' as const },
      compatibility: status.compatibility,
      passiveStatus: {
        status: 'available' as const,
        source: 'signed-server-status' as const,
      },
      services: { chat: status.chatSupported },
      restrictions: allRestrictions,
    };
  });
  const observedExpiredLeases = (base?.observedExpiredLeases ?? []).filter(
    (entry) => {
      const server = servers.find((server) => server.id === entry.profile);
      const previous = base?.servers.find(
        (server) => server.id === entry.profile,
      );
      return (
        server &&
        previous &&
        server.configuredProbe === previous.configuredProbe &&
        (!server.host_id ||
          !previous.host_id ||
          server.host_id === previous.host_id)
      );
    },
  );
  for (const server of servers) {
    const lease = server.compatibility;
    if (
      (lease.status === 'required' || lease.status === 'incompatible') &&
      lease.expiresAt <= nowSeconds &&
      !observedExpiredLeases.some(
        (entry) =>
          entry.profile === server.id && entry.expiresAt === lease.expiresAt,
      )
    )
      observedExpiredLeases.push({
        profile: server.id,
        expiresAt: lease.expiresAt,
      });
  }
  const sameIdentityStores = new Set(
    stores
      .filter((store) => {
        const previous = base?.stores.find((entry) => entry.id === store.id);
        const server = servers.find((entry) => entry.id === store.server);
        const previousServer = base?.servers.find(
          (entry) => entry.id === store.server,
        );
        return (
          previous &&
          sameStoreIdentity(previous, store) &&
          server &&
          previousServer &&
          server.configuredProbe === previousServer.configuredProbe &&
          (!server.host_id ||
            !previousServer.host_id ||
            server.host_id === previousServer.host_id)
        );
      })
      .map((store) => store.id),
  );
  for (const [index, inventory] of storeInventory.entries()) {
    const store = stores.find((store) => store.id === inventory.store)!;
    if (
      partial &&
      !catalogStoreComplete(response, store, partial) &&
      !sameIdentityStores.has(store.id) &&
      inventory.status === 'available'
    )
      storeInventory[index] = {
        store: store.id,
        status: 'loading',
        restrictions: [],
      };
  }
  const rawAccounts = embeddedMetadata
    ? (response.localMetadata?.accounts ?? [])
    : (await bridge.listAccounts(response.generation)).filter(
        (account) =>
          profileScope === undefined || account.server === profileScope,
      );
  const unavailableServers = new Set(
    servers
      .filter(
        (server) =>
          !serverFactAvailability(server, observedExpiredLeases, { nowSeconds })
            .available,
      )
      .map((server) => server.id),
  );
  // Inactive teams cannot query members or federation until setup is complete;
  // skip those reads.
  const teams = liveStores.filter(
    (store) =>
      !partial &&
      store.kind === 'team' &&
      store.active &&
      !blockedProfiles.has(store.server) &&
      !unavailableServers.has(store.server) &&
      servers.some(
        (server) =>
          server.id === store.server &&
          serverFactAvailability(
            server,
            observedExpiredLeases,
            { nowSeconds },
            ['teams'],
          ).available,
      ),
  );
  // An invitation decision moves the team chain, but the panel that made it
  // knows before the agent's next catalog read carries the new sequence.
  const staleRosterProfiles = new Set(
    [...new Set(teams.map((store) => store.server))].filter(
      consumeProfileRosterStaleness,
    ),
  );
  /**
   * The roster this snapshot's base already holds for a team whose chain has
   * not moved, or `undefined` when the roster must be read: no base roster,
   * an unknown or changed chain sequence, a store read that did not succeed
   * this cycle, a base whose store identity differs, or a base roster that
   * recorded a failure.
   *
   * Accepted staleness: a member's username or device change does not move
   * the team chain, so a reused roster shows the previous one until the
   * chain moves, the profile is refreshed after a write, or the user presses
   * Refresh. Rosters here are display and target-selection facts; every
   * roster-changing native operation reloads the team itself before it acts.
   */
  const cachedRoster = (
    store: Store,
  ): { parties: Party[]; federation: FederationEntry[] } | undefined => {
    if (forceRosters || staleRosterProfiles.has(store.server)) return undefined;
    if (store.kind !== 'team' || store.chain_seqno === undefined)
      return undefined;
    // The sequence only moves when a team load succeeds. A team whose store
    // read did not succeed this cycle would otherwise hold its sequence, and
    // its last roster, for as long as the read keeps failing, with no
    // failure band to say so. Read it and let the read report.
    if (!catalogStoreComplete(response, store, partial)) return undefined;
    const previous = base?.stores.find((entry) => entry.id === store.id);
    if (
      !previous ||
      previous.kind !== 'team' ||
      !sameStoreIdentity(previous, store) ||
      previous.chain_seqno !== store.chain_seqno
    )
      return undefined;
    if (base?.groupDetailFailures.some((entry) => entry.store === store.id))
      return undefined;
    const parties = (base?.parties ?? []).filter(
      (party) => party.store === store.id,
    );
    // A roster a read produced always names at least the reader, so no party
    // at all means this snapshot holds no roster for the team, not an empty
    // one. Federation entries are legitimately absent.
    if (parties.length === 0) return undefined;
    return {
      parties,
      federation: (base?.federation ?? []).filter(
        (entry) => entry.store === store.id,
      ),
    };
  };
  const rosters = await Promise.all(
    teams.map(async (store) => {
      const cached = cachedRoster(store);
      if (cached) return { ...cached, failures: [] as GroupDetailFailure[] };
      counts.rosters++;
      const rosterActivity = activity?.child('Loading team rosters');
      const { parties, federation, failures } = await scheduleProfileWork(
        bridge,
        store.server,
        async () => {
          let parties: Party[] = [];
          let federation: FederationEntry[] = [];
          const failures: GroupDetailFailure[] = [];
          try {
            const details = await bridge.listGroupDetails(store.id);
            if (details.parties.status === 'success')
              parties = details.parties.value;
            else
              failures.push(
                recoverableGroupDetailFailure(
                  details.parties.error,
                  store.id,
                  'roster',
                ),
              );
            if (details.federation.status === 'success')
              federation = details.federation.value;
            else
              failures.push(
                recoverableGroupDetailFailure(
                  details.federation.error,
                  store.id,
                  'federation',
                ),
              );
          } catch (error) {
            // Record a complete group-detail failure once under the roster.
            // Federation consumers also check roster failures, so a second entry
            // would duplicate the status banner and notification.
            failures.push(
              recoverableGroupDetailFailure(error, store.id, 'roster'),
            );
          }
          return { parties, federation, failures };
        },
        background
          ? { ...background, key: `${background.key}:roster:${store.id}` }
          : undefined,
      )
        .catch((error: unknown) => {
          // Admission itself failed: the profile's queue was full, or the wait
          // for it ran out. That is one team's roster this refresh could not
          // read, recorded like any other roster failure, not a reason to
          // discard the whole catalog. A cancellation is this refresh's own
          // retirement and stays fatal to it.
          if (normalizeCommandError(error).code === 'cancelled') throw error;
          return {
            parties: [] as Party[],
            federation: [] as FederationEntry[],
            failures: [
              recoverableGroupDetailFailure(error, store.id, 'roster'),
            ],
          };
        })
        .finally(() => rosterActivity?.finish());
      if (parties.some((party) => party.store !== store.id)) {
        throw new Error(
          'list_parties returned a roster for a different store.',
        );
      }
      if (federation.some((entry) => entry.store !== store.id)) {
        throw new Error(
          'list_federation returned group memberships for a different store.',
        );
      }
      return { parties, federation, failures };
    }),
  );
  const groupDetailFailures = rosters.flatMap((roster) => roster.failures);
  const storeIds = new Set(liveStores.map((store) => store.id));
  if (response.items.some((item) => !storeIds.has(item.store))) {
    throw new Error(
      'list_catalog returned an item for a store it did not include.',
    );
  }
  const availableAccountStoreIds = new Set(
    liveStores
      .filter(
        (store) =>
          store.kind === 'account' &&
          !blockedProfiles.has(store.server) &&
          !unavailableServers.has(store.server),
      )
      .map((store) => store.id),
  );
  const accounts = rawAccounts.filter(
    (account) =>
      account.store !== undefined &&
      availableAccountStoreIds.has(account.store),
  );
  if (
    rawAccounts.some(
      (account) => !account.store || !storeIds.has(account.store),
    ) ||
    rawAccounts.some((account) => {
      const store = liveStores.find(
        (candidate) => candidate.id === account.store,
      );
      return store?.kind !== 'account';
    }) ||
    new Set(rawAccounts.map((account) => account.store)).size !==
      rawAccounts.length
  ) {
    throw new Error(
      'list_accounts returned an unknown, non-account, or duplicate store.',
    );
  }
  if (partial) {
    for (const account of base?.accounts ?? []) {
      if (
        sameIdentityStores.has(account.store) &&
        !accounts.some((entry) => entry.store === account.store)
      )
        accounts.push(account);
    }
  }
  if (!partial && accounts.length !== availableAccountStoreIds.size) {
    throw new Error('list_accounts omitted an available account store.');
  }
  const serverIds = new Set(servers.map((server) => server.id));
  if (stores.some((store) => !serverIds.has(store.server))) {
    throw new Error('list_servers omitted a server used by the catalog.');
  }
  const federation = rosters.flatMap((roster) => roster.federation);
  const displayServerById = (profile: string): string => {
    const server = servers.find((candidate) => candidate.id === profile);
    return server ? serverDisplayName(server) : profile;
  };
  const parties: Party[] = rosters
    .flatMap((roster) => roster.parties)
    .map((party) => {
      const team = liveStores.find(
        (store) => store.id === party.store && store.kind === 'team',
      );
      const ownerStore = team
        ? liveStores.find(
            (store) =>
              store.kind === 'account' &&
              store.account === team.account &&
              store.server === team.server,
          )
        : undefined;
      const ownerAccount = ownerStore
        ? accounts.find((account) => account.store === ownerStore.id)
        : undefined;
      const admissions =
        party.party_kind === 'named-team'
          ? federation.filter(
              (entry) =>
                entry.store === party.store &&
                entry.remote_team_id_hex === party.party_id_hex &&
                (!party.scoped_host_id_hex ||
                  entry.remote_host_id_hex === party.scoped_host_id_hex),
            )
          : [];
      return {
        ...party,
        label:
          party.party_kind === 'user' &&
          ownerAccount &&
          party.locally_manageable &&
          !party.scoped_host_id_hex &&
          party.username === ownerAccount.username
            ? 'you'
            : bridge.native
              ? undefined
              : party.label,
        // Resolve the remote server's display name rather than its internal profile identifier.
        team_name:
          admissions.length === 1
            ? `${admissions[0].remote_team_alias} @ ${displayServerById(admissions[0].remote_profile)}`
            : party.team_name,
      };
    });
  if (partial) {
    parties.push(
      ...(base?.parties ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
    federation.push(
      ...(base?.federation ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
    groupDetailFailures.push(
      ...(base?.groupDetailFailures ?? []).filter((entry) =>
        sameIdentityStores.has(entry.store),
      ),
    );
  }
  const baseItems = new Map(
    (base?.items ?? []).map((item) => [itemKey(item), item]),
  );
  // The item filters below ask the same questions of a store once per item it
  // holds. Both answers are per store, so they are settled here, after the
  // inventory rewrite above has put a still-loading store's status back to
  // 'loading' — built before it, that store's items would be admitted.
  const inventoryByStore = new Map(
    storeInventory.map((entry) => [entry.store, entry]),
  );
  const availableStores = new Set<string>();
  {
    const decided = new Set<string>();
    for (const store of liveStores) {
      // The filter this replaces resolved the store with `find`, so a
      // repeated id is answered by its first record here too.
      if (decided.has(store.id)) continue;
      decided.add(store.id);
      if (
        !unavailableServers.has(store.server) &&
        servers.some(
          (server) =>
            server.id === store.server &&
            serverFactAvailability(
              server,
              observedExpiredLeases,
              { nowSeconds },
              store.kind === 'team' ? ['teams', 'kv'] : ['kv'],
            ).available,
        ) &&
        inventoryByStore.get(store.id)?.status === 'available'
      )
        availableStores.add(store.id);
    }
  }
  const items: Item[] = response.items
    .filter((item) => availableStores.has(item.store))
    .map((item) => ({
      ...(bridge.native ? {} : baseItems.get(itemKey(item))),
      ...item,
    }));
  const itemKeys = new Set(items.map(itemKey));
  for (const item of base?.items ?? []) {
    const store = storesById.get(item.store);
    if (
      !store ||
      !sameIdentityStores.has(item.store) ||
      itemKeys.has(itemKey(item))
    )
      continue;
    if (
      catalogStoreComplete(response, store, partial) &&
      inventoryByStore.get(item.store)?.status === 'available'
    )
      continue;
    const metadata = { ...item };
    delete metadata.value;
    delete metadata.target;
    items.push(bridge.native ? metadata : item);
  }
  const withFreshness = (snapshot: AgentSnapshot): AgentSnapshot => ({
    ...snapshot,
    servers: snapshot.servers.map((server) => ({
      ...server,
      connectivity: retainProfileConnection(
        server,
        base?.servers.find((previous) => previous.id === server.id)
          ?.connectivity,
      ),
    })),
    observedExpiredLeases,
    catalogFreshness: projectCatalogFreshness(
      snapshot,
      base,
      response,
      partial,
      nowSeconds,
      profileScope,
    ),
  });
  if (!bridge.native) {
    if (!base)
      throw new Error('The mock bridge did not supply its fixture snapshot.');
    return withFreshness({
      ...base,
      agent,
      servers,
      accounts,
      stores,
      storeInventory,
      profileInventory,
      catalogProfiles: response.profiles,
      profileInventoryStatus: 'complete',
      items,
      parties,
      federation,
      groupDetailFailures,
    });
  }
  return withFreshness({
    agent,
    servers,
    accounts,
    stores,
    storeInventory,
    profileInventory,
    catalogProfiles: response.profiles,
    profileInventoryStatus: 'complete',
    items,
    parties,
    federation,
    groupDetailFailures,
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [
      ...notificationsOf(response, servers, nowSeconds).filter(
        (note) =>
          ![...statusFailures.keys()].some(
            (profile) => note.id === `server-status-unavailable-${profile}`,
          ),
      ),
      ...[...statusFailures]
        .filter(([, error]) => error.code !== 'catalog-loading')
        .map(([profile, error]) => ({
          id: `status-unavailable-${profile}`,
          profile,
          severity: 'crit' as const,
          title: `Status for ${profile} is unavailable`,
          detail: `${error.message} Server contents are unavailable until the connection status is verified.${
            incompatibleSchemaProfiles.has(profile)
              ? ` ${schemaInstruction(profile)}`
              : ''
          }`,
          action: 'Inspect',
        })),
      ...groupDetailFailures.map((failure) => ({
        id: `group-${failure.source}-unavailable-${failure.store}`,
        profile: stores.find((store) => store.id === failure.store)?.server,
        severity: 'warn' as const,
        title:
          failure.source === 'roster'
            ? 'Team member list is unavailable'
            : 'Team shared access is unavailable',
        detail: failure.message,
        action: failure.retryable ? 'Refresh' : 'Inspect',
      })),
    ],
    observedExpiredLeases,
    plaintext: {},
  });
}
