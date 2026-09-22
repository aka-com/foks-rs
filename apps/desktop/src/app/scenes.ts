import { decodeForRuntimeScene, INITIAL_SCENE } from '../location';
import type { Bridge } from '../bridge';
import type { Scene } from '../location';
import { applyLease, isLogin, kindOf, nameOf, storeOf } from '../model';
import type { AgentSnapshot, Item } from '../model';

export function fixtureScenesAllowed(
  bridge?: Pick<Bridge, 'native' | 'fixtureSnapshot' | 'firstRunFixture'>,
): boolean {
  return Boolean(
    bridge &&
    (!bridge.native || bridge.fixtureSnapshot || bridge.firstRunFixture),
  );
}

/** The scene the address bar asks for, or the shell's own starting point. */
export function initialScene(
  bridge?: Pick<Bridge, 'native' | 'fixtureSnapshot' | 'firstRunFixture'>,
): Scene {
  if (typeof window === 'undefined') return INITIAL_SCENE;
  return decodeForRuntimeScene(window.location.search, {
    fixtures: fixtureScenesAllowed(bridge),
  });
}

/** Applies review-scene failures once at the fixture/model boundary. */
export function demoAvailabilityFacts(
  agentSnapshot: AgentSnapshot,
  state: string,
): AgentSnapshot {
  if (
    state === 'servers-list' ||
    state === 'servers-add' ||
    state === 'servers-lapsed'
  )
    return applyLease(agentSnapshot, 'lapsed');
  if (state === 'servers-rollback') {
    const error = {
      code: 'host-verification-failed',
      message: 'The fixture host identity moved backwards.',
      fatal: false,
      retryable: false,
      ambiguous: false,
    };
    return {
      ...agentSnapshot,
      servers: agentSnapshot.servers.map((server) =>
        server.profileName === 'personal'
          ? { ...server, trust: { status: 'blocked' as const, error } }
          : server,
      ),
    };
  }
  if (state === 'settings-account') {
    const error = {
      code: 'catalog-unavailable',
      message: 'The fixture vault inventory is unavailable.',
      fatal: false,
      retryable: true,
      ambiguous: false,
    };
    return {
      ...agentSnapshot,
      storeInventory: agentSnapshot.storeInventory.map((entry) =>
        entry.store === 'acct:work'
          ? { ...entry, status: 'unavailable' as const, error }
          : entry,
      ),
    };
  }
  return agentSnapshot;
}

export function demoSelection(
  demo: Scene['demo'],
  agentSnapshot: AgentSnapshot,
): { store: string; path: string } | null {
  if (!demo) return null;
  const candidates = agentSnapshot.items.filter(
    (item) => item.kind !== 'Folder',
  );
  let item: Item | undefined;
  if (demo === 'password')
    item = candidates.find((candidate) => isLogin(candidate));
  else if (demo === 'resource') {
    item = candidates
      .filter(
        (candidate) =>
          candidate.kind === 'Secret' &&
          kindOf(candidate) === 'Document' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'account',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  } else if (demo === 'file') {
    item = candidates
      .filter(
        (candidate) =>
          candidate.kind === 'File' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'team',
      )
      .sort((left, right) => right.version - left.version)[0];
  } else {
    item = candidates
      .filter(
        (candidate) =>
          kindOf(candidate) === 'Password' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'team',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  }
  return item ? { store: item.store, path: item.path } : null;
}
