import assert from 'node:assert/strict';
import test from 'node:test';
import { loadSnapshot, type Bridge, type CatalogDto } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import {
  markProfileRostersStale,
  resetProfileRosterStaleness,
} from '../src/roster-staleness';
import type { AgentSnapshot } from '../src/model';

/**
 * A catalog whose team stores all report `seqno` as their pinned team chain
 * sequence, or none at all when it is undefined.
 */
function catalogWithSeqno(catalog: CatalogDto, seqno?: number): CatalogDto {
  return {
    ...catalog,
    stores: catalog.stores.map((store) =>
      store.kind === 'team' ? { ...store, chain_seqno: seqno } : store,
    ),
  };
}

async function harness() {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  let current = catalogWithSeqno(catalog, 4);
  const reads: string[] = [];
  const bridge: Bridge = {
    ...base,
    listCatalog: async () => current,
    listGroupDetails: async (store) => {
      reads.push(store);
      return base.listGroupDetails(store);
    },
  };
  /** Reports every store as read, except `store`, whose read failed. */
  const failStoreRead = (store: string) => {
    current = {
      ...current,
      storeReads: current.stores.map((entry) => ({
        store: entry.id,
        state: entry.id === store ? ('failed' as const) : ('complete' as const),
      })),
    };
  };
  const load = (
    snapshot: AgentSnapshot | undefined,
    forceRosters = false,
  ): Promise<AgentSnapshot> =>
    loadSnapshot(bridge, snapshot, 1, undefined, () => true, forceRosters);
  return {
    reads,
    load,
    failStoreRead,
    setCatalog: (seqno?: number) => {
      current = catalogWithSeqno(catalog, seqno);
    },
  };
}

test('an unchanged team chain sequence reuses the roster the snapshot already holds', async () => {
  resetProfileRosterStaleness();
  const { reads, load } = await harness();
  const first = await load(FIXTURE);
  assert.ok(reads.length > 0);
  assert.ok(first.parties.length > 0);
  reads.length = 0;
  const again = await load(first);
  assert.deepEqual(reads, []);
  assert.deepEqual(again.parties, first.parties);
  assert.deepEqual(again.federation, first.federation);
  assert.deepEqual(again.groupDetailFailures, []);
});

test('a moved chain sequence, an unknown one, and a forced read all read the roster', async () => {
  resetProfileRosterStaleness();
  const { reads, load, setCatalog } = await harness();
  const first = await load(FIXTURE);
  const teams = reads.length;
  // The chain moved: the roster this snapshot holds may be out of date.
  reads.length = 0;
  setCatalog(5);
  const moved = await load(first);
  assert.equal(reads.length, teams);
  // A refresh the user asked for reads every roster whatever the sequence.
  reads.length = 0;
  await load(moved, true);
  assert.equal(reads.length, teams);
  // An agent that reports no sequence says "unknown", never "unchanged".
  reads.length = 0;
  setCatalog(undefined);
  const unknown = await load(moved);
  assert.equal(reads.length, teams);
  reads.length = 0;
  await load(unknown);
  assert.equal(reads.length, teams);
});

test('a roster the snapshot never held, or held as a failure, is read again', async () => {
  resetProfileRosterStaleness();
  const { reads, load } = await harness();
  const first = await load(FIXTURE);
  const teams = reads.length;
  // No cached roster for a team: its parties are missing from the base.
  reads.length = 0;
  const store = first.parties[0].store;
  await load({
    ...first,
    parties: first.parties.filter((party) => party.store !== store),
  });
  assert.deepEqual(reads, [store]);
  // A cached roster that recorded a failure is not a roster to reuse.
  reads.length = 0;
  await load({
    ...first,
    groupDetailFailures: [
      {
        store,
        source: 'roster',
        code: 'io',
        message: 'The roster could not be read.',
        retryable: true,
      },
    ],
  });
  assert.deepEqual(reads, [store]);
  reads.length = 0;
  await load(first);
  assert.equal(reads.length, 0);
  assert.equal(teams > 0, true);
});

test('invitation activity reads a profile’s rosters once more', async () => {
  resetProfileRosterStaleness();
  const { reads, load } = await harness();
  const first = await load(FIXTURE);
  reads.length = 0;
  const profile = first.stores.find((store) => store.kind === 'team')!.server;
  markProfileRostersStale(profile);
  const after = await load(first);
  assert.ok(reads.length > 0);
  assert.ok(
    reads.every(
      (read) =>
        first.stores.find((store) => store.id === read)?.server === profile,
    ),
  );
  // The mark is consumed by the read it forced, not left standing.
  reads.length = 0;
  await load(after);
  assert.deepEqual(reads, []);
});

test('a team whose store read failed this cycle is read again, unchanged sequence or not', async () => {
  resetProfileRosterStaleness();
  const { reads, load, failStoreRead } = await harness();
  const first = await load(FIXTURE);
  reads.length = 0;
  // The chain sequence only moves when a team load succeeds. A team whose
  // read keeps failing would otherwise hold its sequence, and its last
  // roster, forever, with nothing on screen to say so.
  const store = first.parties[0].store;
  failStoreRead(store);
  const after = await load(first);
  assert.deepEqual(reads, [store]);
  assert.ok(after.parties.some((party) => party.store === store));
});
