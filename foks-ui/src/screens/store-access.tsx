/** Shared full-page treatment for a store whose contents are unavailable. */

import type { ReactNode } from 'react';
import { Button, Chip, Notice } from '../components';
import { serverOf, storeDescription, storeDescriptionState } from '../model';
import type { Store, StoreDescriptionState, World } from '../model';
import { PageHeader } from '../shell/page-header';

type AccessProblem = Exclude<StoreDescriptionState, 'normal'>;

interface StoreAccessCopy {
  title: string;
  detail: string;
  action: 'open-server' | 'review-server' | 'finish-setup';
  status?: string;
}

function unavailableSubject(store: Store): string {
  return store.kind === 'team'
    ? `Items and members in ${store.name} are unavailable`
    : `Items in ${store.name} are unavailable`;
}

function accessCopy(
  state: AccessProblem,
  store: Store,
  serverName: string,
): StoreAccessCopy {
  const subject = unavailableSubject(store);
  switch (state) {
    case 'blocked':
      return {
        title: 'Server access blocked',
        detail: `A protocol safety check blocked ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'lease-unavailable':
      return {
        title: 'Server status unavailable',
        detail: `FOKS cannot verify the compatibility lease for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'lease-lapsed':
      return {
        title: 'Compatibility lease expired',
        detail: `${subject} until the agent renews the lease for ${serverName}.`,
        action: 'open-server',
        status: 'Waiting for renewal',
      };
    case 'inactive':
      return {
        title: 'Group setup incomplete',
        detail: 'Items and members are unavailable until setup is finished.',
        action: 'finish-setup',
      };
  }
}

export interface StoreAccessTakeoverProps {
  world: World;
  store: Store;
  lead?: ReactNode;
  onOpenServer: (profile: string) => void;
  onFinishSetup: () => void;
}

export function StoreAccessTakeover({
  world,
  store,
  lead,
  onOpenServer,
  onFinishSetup,
}: StoreAccessTakeoverProps): ReactNode {
  const state = storeDescriptionState(world, store);
  if (state === 'normal') return null;
  const server = serverOf(world, store.id);
  const copy = accessCopy(state, store, server?.name ?? store.server);
  const action =
    copy.action === 'finish-setup' ? (
      <Button variant="primary" onClick={onFinishSetup}>
        Finish setup
      </Button>
    ) : (
      <Button onClick={() => onOpenServer(server?.id ?? store.server)}>
        {copy.action === 'open-server' ? 'Open server' : 'Review server'}
      </Button>
    );

  return (
    <>
      <PageHeader
        title={store.name}
        subtitle={storeDescription(world, store)}
        lead={lead}
      />
      <div className="body">
        <Notice
          severity={state === 'inactive' ? 'warn' : 'crit'}
          title={copy.title}
          footnote={
            state === 'inactive' ? undefined : 'Other servers are unaffected.'
          }
          actions={
            <>
              {action}
              {copy.status ? <Chip tone="warn">{copy.status}</Chip> : null}
            </>
          }
        >
          <p>{copy.detail}</p>
        </Notice>
      </div>
    </>
  );
}

export interface StoreAccessBand {
  key: string;
  text: string;
}

function joinNames(stores: readonly Store[]): string {
  const names = stores.map((store) => store.name);
  if (names.length < 2) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** Aggregate unavailable stores without mixing profiles or hiding inactive groups. */
export function storeAccessBands(world: World): StoreAccessBand[] {
  const buckets = new Map<string, { state: AccessProblem; stores: Store[] }>();
  for (const store of world.stores) {
    const state = storeDescriptionState(world, store);
    if (state === 'normal') continue;
    const key =
      state === 'inactive'
        ? `inactive:${store.id}`
        : `${state}:${store.server}`;
    const bucket = buckets.get(key) ?? { state, stores: [] };
    bucket.stores.push(store);
    buckets.set(key, bucket);
  }

  return [...buckets.entries()].map(([key, { state, stores }]) => {
    const names = joinNames(stores);
    const verb = stores.length === 1 ? 'is' : 'are';
    const serverName = serverOf(world, stores[0].id)?.name ?? stores[0].server;
    const text =
      state === 'blocked'
        ? `${names} ${verb} unavailable because a protocol safety check blocked ${serverName}.`
        : state === 'lease-unavailable'
          ? `${names} ${verb} unavailable because FOKS cannot verify the compatibility lease for ${serverName}.`
          : state === 'lease-lapsed'
            ? `${names} ${verb} unavailable because the compatibility lease for ${serverName} expired.`
            : `${names} ${verb} unavailable because group setup is incomplete.`;
    return { key, text };
  });
}
