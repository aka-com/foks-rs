import assert from 'node:assert/strict';
import test from 'node:test';

import {
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
