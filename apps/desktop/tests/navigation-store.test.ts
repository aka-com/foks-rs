import assert from 'node:assert/strict';
import test from 'node:test';

import {
  accountAtLocation,
  sameLocation,
  chatTabLocation,
  INITIAL_STATE,
  LocationStore,
  rememberChatLocation,
  rememberedChatRef,
  storeAtScene,
  decodeProductionScene,
  transition,
  useLocationState,
} from '../src/location';
import type {
  GuardVerdict,
  LocationAction,
  LocationState,
} from '../src/location';
import type { AccountStore } from '../src/model/types';
import { LocationStore as LeafStore } from '../src/navigation/location-store';
import { transition as leafTransition } from '../src/navigation/transition';
import { useLocationState as leafHook } from '../src/navigation/use-location-state';
import { chatTabLocation as leafChatLocation } from '../src/navigation/chat-tab-memory';

const ACCOUNT_A: AccountStore = {
  id: 'account-a',
  kind: 'account',
  name: 'A',
  server: 'a',
  account: 'owner',
};
const ACCOUNT_B: AccountStore = {
  id: 'account-b',
  kind: 'account',
  name: 'B',
  server: 'b',
  account: 'owner',
};
const PROMPT: Extract<GuardVerdict, { verdict: 'prompt' }> = {
  verdict: 'prompt',
  title: 'Discard?',
  body: 'Unsaved draft',
  confirm: 'Discard',
};

test('facade exports the same store, reducer, hook and chat memory as their leaf modules', () => {
  assert.equal(LocationStore, LeafStore);
  assert.equal(transition, leafTransition);
  assert.equal(useLocationState, leafHook);
  assert.equal(chatTabLocation, leafChatLocation);
});

test('external store snapshots and bound subscription methods remain stable', () => {
  const store = new LeafStore();
  const { getSnapshot, subscribe } = store;
  const first = getSnapshot();
  const snapshots: LocationState[] = [];
  const unsubscribe = subscribe(() => snapshots.push(getSnapshot()));
  const noops: LocationAction[] = [
    { type: 'navigate', location: { kind: 'all' } },
    { type: 'search', query: '' },
    { type: 'view', view: 'list' },
    { type: 'details', open: false },
    { type: 'kind', kind: 'All' },
    { type: 'sort', sort: 'name' },
    { type: 'folder', folder: '' },
  ];
  for (const action of noops) {
    assert.equal(store.dispatch(action), first);
    assert.equal(getSnapshot(), first);
  }
  assert.deepEqual(snapshots, []);
  store.search('secret');
  const next = getSnapshot();
  assert.notEqual(next, first);
  assert.equal(next.query, 'secret');
  assert.equal(getSnapshot(), next);
  assert.equal(store.getSnapshot, getSnapshot);
  assert.equal(store.subscribe, subscribe);
  assert.deepEqual(snapshots, [next]);
  unsubscribe();
  store.search('other');
  assert.deepEqual(snapshots, [next]);
});

test('storeAtScene preserves decoded navigation without introducing fixture state', () => {
  const scene = decodeProductionScene(
    '?state=show&sel=live|/key&kind=Password&lease=lapsed',
  );
  const store = storeAtScene(scene);
  const snapshot = store.getSnapshot();
  assert.equal(snapshot.selection, scene.selection);
  assert.equal(snapshot.details, true);
  assert.equal(snapshot.kind, 'Password');
  assert.equal(store.getSnapshot(), snapshot);
  assert.deepEqual(snapshot.location, { kind: 'all' });
  assert.equal(scene.reveal, false);
});

test('tab memory restores view state and draft but never a previous selection', () => {
  const store = new LocationStore();
  store.navigate({ kind: 'store', ref: 'live-team' });
  store.setFolder('/documents');
  store.toggleFolder('/documents/private');
  store.search('invoice');
  store.setKind('Document');
  store.setSort('group');
  store.setSheetField('draft', 'saved');
  store.select({ store: 'live-team', path: '/documents/invoice' });
  const files = store.getSnapshot();
  store.navigateTab('teams');
  assert.equal(store.getSnapshot().query, '');
  store.navigateTab('files');
  assert.deepEqual(store.getSnapshot(), {
    ...files,
    selection: null,
    details: false,
  });
  const resumed = store.getSnapshot();
  store.navigateTab('files');
  assert.equal(store.getSnapshot(), resumed);
  store.clearTabMemory();
  store.navigateTab('teams');
  store.navigateTab('files');
  assert.equal(store.getSnapshot().sheet, undefined);
});

test('chat memory is shared with the facade and removed inventory invalidates the target', () => {
  rememberChatLocation({
    kind: 'chat',
    ref: 'live-team',
    channel: 'ab'.repeat(16),
  });
  try {
    assert.equal(rememberedChatRef(), 'live-team');
    const store = new LeafStore();
    store.navigateTab('chat');
    assert.deepEqual(store.getSnapshot().location, chatTabLocation());
    store.navigateTab('files');
    store.setAccountStores([]);
    store.navigateTab('chat');
    assert.deepEqual(store.getSnapshot().location, { kind: 'chat' });
  } finally {
    rememberChatLocation(null);
  }
  assert.equal(rememberedChatRef(), undefined);
});

test('a refused tab switch does not mutate snapshots, drafts or the acting account', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'devices', store: ACCOUNT_A.id, device: 'mac-a' },
  });
  store.setAccountStores([ACCOUNT_A, ACCOUNT_B]);
  store.setSheetField('draft', 'keep');
  store.setSheetRestorable(Symbol('editor'), false);
  const first = store.getSnapshot();
  const refusals: string[] = [];
  store.setRefusalHandler((reason) => refusals.push(reason));
  store.registerGuard(() => PROMPT);
  const unregister = store.registerGuard(() => ({
    verdict: 'refuse',
    reason: 'busy',
  }));
  store.navigateTab('settings');
  assert.equal(store.getSnapshot(), first);
  assert.equal(store.getAccount(), ACCOUNT_A.id);
  assert.deepEqual(refusals, ['busy']);
  unregister();
  store.navigate(
    { kind: 'settings', section: 'account', store: ACCOUNT_B.id },
    { force: true },
  );
  assert.equal(store.getAccount(), ACCOUNT_B.id);
  assert.equal(store.getSnapshot().sheet, undefined);
  store.navigateTab('devices', { force: true });
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'devices',
    store: ACCOUNT_B.id,
  });
});

test('superseded prompts do not run callbacks and navigateAndSelect stays guarded', async () => {
  const store = new LocationStore();
  const resolutions: Array<(confirmed: boolean) => void> = [];
  let confirmed = 0;
  store.registerGuard(() => ({
    ...PROMPT,
    onConfirm: () => {
      confirmed += 1;
    },
  }));
  store.setPrompter(
    () => new Promise<boolean>((resolve) => resolutions.push(resolve)),
  );
  const initial = store.getSnapshot();
  store.navigateAndSelect(
    { kind: 'store', ref: 'team-a' },
    { store: 'team-a', path: '/secret' },
  );
  assert.equal(store.getSnapshot(), initial);
  store.navigate({ kind: 'settings', section: 'account' });
  resolutions[0](true);
  await Promise.resolve();
  assert.equal(store.getSnapshot(), initial);
  assert.equal(confirmed, 0);
  resolutions[1](true);
  await Promise.resolve();
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'settings',
    section: 'account',
  });
  assert.equal(store.getSnapshot().selection, null);
  assert.equal(confirmed, 1);
  store.navigate({ kind: 'teams' });
  store.navigate({ kind: 'files' }, { force: true });
  resolutions[2](true);
  await Promise.resolve();
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  assert.equal(confirmed, 1);
});

test('Back follows cross-tab visits and restores page state without cycling', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.search('token');
  store.select({ store: ACCOUNT_A.id, path: '/token' });
  const files = store.getSnapshot();
  store.navigate({ kind: 'settings', section: 'account', store: ACCOUNT_A.id });
  const settings = store.getSnapshot().location;
  store.navigate({ kind: 'devices', store: ACCOUNT_A.id });
  store.back();
  assert.deepEqual(store.getSnapshot().location, settings);
  store.navigate({ kind: 'teams', store: ACCOUNT_A.id });
  store.back();
  assert.deepEqual(store.getSnapshot().location, settings);
  store.back();
  assert.deepEqual(store.getSnapshot(), { ...files, sheet: undefined });
  assert.equal(store.backTarget(), null);
  store.back();
  assert.equal(store.getSnapshot().location.kind, 'files');
});

test('Back ignores replacements, preserves history on refusal, and clears at session reset', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.navigate({ kind: 'chat' });
  store.navigate(
    { kind: 'chat', ref: 'team:eng', channel: 'general' },
    { replace: true },
  );
  assert.deepEqual(store.backTarget(), { kind: 'files' });
  const unregister = store.registerGuard(() => ({
    verdict: 'refuse',
    reason: 'Saving',
  }));
  store.back();
  assert.equal(store.getSnapshot().location.kind, 'chat');
  assert.deepEqual(store.backTarget(), { kind: 'files' });
  unregister();
  store.back();
  assert.equal(store.getSnapshot().location.kind, 'files');
  store.navigateTab('settings');
  store.clearTabMemory();
  assert.equal(store.backTarget(), null);
});

test('Back restores the previous Files folder without restoring transient sheets', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.setFolder('account-a|/one');
  store.setSheetField('draft', 'temporary');
  store.setFolder('account-a|/two');
  store.back();
  assert.equal(store.getSnapshot().folder, 'account-a|/one');
  assert.equal(store.getSnapshot().sheet, undefined);
});

test('first-run blocks history Back even with earlier visits and force enabled', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.navigate({ kind: 'settings' });
  store.navigate({ kind: 'first-run', step: 'address' });
  assert.equal(store.backTarget(), null);
  store.back({ force: true });
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'first-run',
    step: 'address',
  });
});

test('a Teams sheet intent is spent where it was handed over, not remembered', () => {
  const store = new LocationStore();
  // The Chat tab's empty pane sends a reader to Teams to create a team.
  store.navigate({ kind: 'teams', open: 'create' });
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'teams',
    open: 'create',
  });
  // The page consumes the intent and canonicalizes its own address.
  store.navigate({ kind: 'teams' }, { replace: true, force: true });
  assert.deepEqual(store.getSnapshot().location, { kind: 'teams' });
  store.navigateTab('files');
  store.navigateTab('teams');
  // The resumed address names the acting account, which this bare store has
  // none of; what matters is that no sheet is asked for.
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'teams',
    store: undefined,
  });
  // Even a tab left before the page could consume the intent resumes the
  // list: the sheet belongs to the arrival, not to the tab.
  store.navigate({ kind: 'teams', open: 'join' });
  store.navigateTab('files');
  store.navigateTab('teams');
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'teams',
    store: undefined,
  });
});

test('a Teams sheet opened on the canonical address survives a tab round trip', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'teams' },
  });
  store.setAccountStores([ACCOUNT_A]);
  store.setSheetField('teams.sheet', { kind: 'create', store: ACCOUNT_A.id });
  store.navigateTab('files');
  store.navigateTab('teams');
  assert.deepEqual(store.getSnapshot().sheet, {
    'teams.sheet': { kind: 'create', store: ACCOUNT_A.id },
  });
});

test('a search survives a move inside one rail tab and is dropped on leaving it', () => {
  // Searching the Chat header and then opening a channel the search matched
  // keeps the query: the reader is still in the results they typed for.
  const opened = leafTransition(
    { ...INITIAL_STATE, location: { kind: 'chat' }, query: 'deploys' },
    {
      type: 'navigate',
      location: { kind: 'chat', ref: 'team', channel: 'c1' },
    },
  );
  assert.equal(opened.query, 'deploys');
  // The tab canonicalizes `{kind:'chat'}` into the conversation it stands for
  // at mount; that move is inside the tab as well, so it keeps the query too.
  const canonical = leafTransition(
    { ...INITIAL_STATE, location: { kind: 'chat' }, query: 'deploys' },
    { type: 'navigate', location: { kind: 'chat', ref: 'team' } },
  );
  assert.equal(canonical.query, 'deploys');
  // Leaving the tab clears it: the next tab's field is its own.
  const left = leafTransition(
    {
      ...INITIAL_STATE,
      location: { kind: 'chat', ref: 'team' },
      query: 'deploys',
    },
    { type: 'navigate', location: { kind: 'files' } },
  );
  assert.equal(left.query, '');
  // And arriving on Chat from another tab starts empty.
  const arrived = leafTransition(
    { ...INITIAL_STATE, location: { kind: 'files' }, query: 'invoice' },
    { type: 'navigate', location: { kind: 'chat', ref: 'team' } },
  );
  assert.equal(arrived.query, '');
  // A move within Files keeps what was typed there, as it always has.
  const within = leafTransition(
    { ...INITIAL_STATE, location: { kind: 'files' }, query: 'invoice' },
    { type: 'navigate', location: { kind: 'store', ref: 'live-team' } },
  );
  assert.equal(within.query, 'invoice');
});

test('Forward restores visits and page state; a new visit clears only the forward branch', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.setFolder('account-a|/one');
  store.navigate({ kind: 'settings' });
  store.search('saved query');
  store.setSheetField('draft', 'not history');
  store.back();
  assert.equal(store.getSnapshot().folder, 'account-a|/one');
  assert.equal(store.forwardTarget()?.kind, 'settings');
  store.search('a different query'); // View edits are not new visits.
  store.forward();
  assert.equal(store.getSnapshot().query, 'saved query');
  assert.equal(store.getSnapshot().sheet, undefined);
  assert.equal(store.forwardTarget(), null);
  store.back();
  store.navigate({ kind: 'teams' });
  assert.equal(store.forwardTarget(), null);
  store.back();
  assert.equal(store.getSnapshot().query, 'a different query');
  store.clearTabMemory();
  assert.equal(store.forwardTarget(), null);
  assert.equal(store.backTarget(), null);
});

test('Forward respects refusal, cancellation, confirmation and superseded prompts', async () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.navigate({ kind: 'settings' });
  store.back();
  let verdict: GuardVerdict = { verdict: 'refuse', reason: 'Saving' };
  store.registerGuard(() => verdict);
  let settle: (confirmed: boolean) => void = () => {};
  store.setPrompter(
    () =>
      new Promise<boolean>((resolve) => {
        settle = resolve;
      }),
  );
  store.forward();
  assert.equal(store.getSnapshot().location.kind, 'files');
  assert.equal(store.forwardTarget()?.kind, 'settings');
  verdict = PROMPT;
  store.forward();
  settle(false);
  await Promise.resolve();
  assert.equal(store.getSnapshot().location.kind, 'files');
  store.forward();
  settle(true);
  await Promise.resolve();
  assert.equal(store.getSnapshot().location.kind, 'settings');
  store.back({ force: true });
  store.forward();
  store.navigate({ kind: 'teams' }, { force: true });
  settle(true);
  await Promise.resolve();
  assert.equal(store.getSnapshot().location.kind, 'teams');
  assert.equal(store.forwardTarget(), null);
});

test('replacement preserves Forward and first-run blocks both directions', () => {
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'files' },
  });
  store.navigate({ kind: 'settings' });
  store.back();
  store.navigate({ kind: 'all' }, { replace: true });
  assert.equal(store.backTarget(), null);
  assert.equal(store.forwardTarget()?.kind, 'settings');
  store.navigate({ kind: 'first-run', step: 'address' });
  store.forward({ force: true });
  assert.equal(store.forwardTarget(), null);
  assert.equal(store.getSnapshot().location.kind, 'first-run');
});

const TEAM_B = {
  id: 'opaque-team-b',
  kind: 'team' as const,
  name: 'B team',
  alias: 'team',
  server: ACCOUNT_B.server,
  account: ACCOUNT_B.account,
  active: true,
  team_kind: 'named' as const,
  team_id_hex: 'ab'.repeat(33),
};

for (const legacy of [false, true]) {
  test(`Teams keeps its owning account through navigation (legacy=${legacy})`, () => {
    const store = new LocationStore();
    store.setAccountStores([ACCOUNT_A, ACCOUNT_B, TEAM_B]);
    store.navigate({ kind: 'chat', ref: TEAM_B.id });
    store.navigate({
      kind: 'teams',
      ...(legacy ? { store: TEAM_B.id } : { ref: TEAM_B.id }),
      open: 'invite',
    });
    assert.equal(store.getAccount(), ACCOUNT_B.id);
    assert.deepEqual(store.getSnapshot().location, {
      kind: 'teams',
      store: ACCOUNT_B.id,
      ref: TEAM_B.id,
      open: 'invite',
    });
    store.navigate({ kind: 'teams', store: ACCOUNT_B.id }, { replace: true });
    store.navigateTab('settings');
    assert.equal(store.getAccount(), ACCOUNT_B.id);
    assert.deepEqual(store.getSnapshot().location, {
      kind: 'settings',
      store: ACCOUNT_B.id,
    });
    store.navigateTab('teams');
    assert.equal(store.getAccount(), ACCOUNT_B.id);
    assert.equal('open' in store.getSnapshot().location, false);
  });
}

test('explicit missing or conflicting Teams identities never fall back to another account', () => {
  const stores = [ACCOUNT_A, ACCOUNT_B, TEAM_B];
  for (const location of [
    { kind: 'teams' as const, store: 'missing' },
    { kind: 'teams' as const, ref: 'missing' },
    { kind: 'teams' as const, store: ACCOUNT_B.id, ref: 'missing' },
    { kind: 'teams' as const, store: ACCOUNT_A.id, ref: TEAM_B.id },
  ])
    assert.equal(accountAtLocation(stores, location, ACCOUNT_A.id), undefined);
  assert.equal(
    accountAtLocation(
      [ACCOUNT_A, TEAM_B],
      { kind: 'teams', store: TEAM_B.id },
      ACCOUNT_A.id,
    ),
    undefined,
  );
  assert.equal(
    accountAtLocation(stores, { kind: 'teams' }, ACCOUNT_B.id)?.id,
    ACCOUNT_B.id,
  );
  const store = new LocationStore();
  store.setAccountStores(stores);
  store.navigate({ kind: 'teams', ref: TEAM_B.id });
  store.setAccountStores([ACCOUNT_A, ACCOUNT_B]);
  assert.equal(store.getAccount(), undefined);
  assert.equal(
    sameLocation(
      { kind: 'teams', ref: TEAM_B.id },
      { kind: 'teams', ref: 'other' },
    ),
    false,
  );
});
