/** Shared full-page treatment for a store whose contents are unavailable. */

import type { ReactNode } from 'react';
import { Button, Notice } from '../components';
import {
  serverOf,
  storeDescriptionState,
  storeHeadingDescription,
} from '../model';
import type { Store, StoreDescriptionState, World } from '../model';
import { PageHeader } from '../shell/page-header';

type AccessProblem = Exclude<StoreDescriptionState, 'normal'>;

interface StoreAccessCopy {
  title: string;
  detail: string;
  action: 'open-server' | 'review-server' | 'finish-setup';
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
        detail: `A security verification failed for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'lease-unavailable':
      return {
        title: 'Server status unavailable',
        detail: `Cannot verify server status for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'never-probed':
      return {
        title: 'Server not checked yet',
        detail: `${serverName} has not been checked. ${subject} until it is verified.`,
        action: 'review-server',
      };
    case 'catalog-unavailable':
      return {
        title: 'Store connection failed',
        detail: `The server connection for ${store.name} could not be established. ${subject}.`,
        action: 'review-server',
      };
    case 'lease-lapsed':
      return {
        title: 'Check-in expired',
        detail: `${subject} until FOKS reconnects to ${serverName}.`,
        action: 'open-server',
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
  headerAction?: ReactNode;
  noHeader?: boolean;
}

export function StoreAccessTakeover({
  world,
  store,
  lead,
  onOpenServer,
  onFinishSetup,
  headerAction,
  noHeader = false,
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
      {noHeader ? null : (
        <PageHeader
          title={store.name}
          subtitle={storeHeadingDescription(world, store)}
          lead={lead}
          action={
            <>
              {state === 'inactive' ? null : action}
              {headerAction}
            </>
          }
        />
      )}
      <div className="body">
        <Notice
          severity={state === 'inactive' ? 'warn' : 'crit'}
          title={copy.title}
          actions={action}
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

/** Aggregate catalog-level access problems. Incomplete group setup stays on the group, not All items. */
export function storeAccessBands(world: World): StoreAccessBand[] {
  const buckets = new Map<string, { state: AccessProblem; stores: Store[] }>();
  for (const store of world.stores) {
    const state = storeDescriptionState(world, store);
    if (state === 'normal' || state === 'inactive') continue;
    const key = `${state}:${store.server}`;
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
        ? `${names} ${verb} unavailable because security verification failed for ${serverName}.`
        : state === 'lease-unavailable'
          ? `${names} ${verb} unavailable because server status could not be verified for ${serverName}.`
          : state === 'lease-lapsed'
            ? `${names} ${verb} unavailable because the connection to ${serverName} expired.`
            : state === 'catalog-unavailable'
              ? `${names} ${verb} unavailable because the latest item list could not be loaded.`
              : `${names} ${verb} unavailable because ${serverName} has not been checked.`;
    return { key, text };
  });
}
