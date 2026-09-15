/** Full-page error and recovery view displayed when a store cannot be accessed. */

import type { ReactNode } from 'react';
import { Band, Button, Notice } from '../components';
import {
  serverOf,
  storeDescriptionState,
  storeHeadingDescription,
} from '../model';
import type { Store, StoreDescriptionState, AgentSnapshot } from '../model';
import { PageHeader } from '../shell/page-header';

type AccessProblem = Exclude<StoreDescriptionState, 'normal'>;

interface StoreAccessCopy {
  title: string;
  detail: string;
  action: 'open-server' | 'review-server' | 'finish-setup';
}

function resourceSubject(store: Store): string {
  return store.kind === 'team'
    ? `items and members in ${store.name}`
    : `items in ${store.name}`;
}

function accessCopy(
  state: AccessProblem,
  store: Store,
  serverName: string,
): StoreAccessCopy {
  const resource = resourceSubject(store);
  const subject = `${resource[0].toUpperCase()}${resource.slice(1)} are unavailable`;
  switch (state) {
    case 'verification-failed':
      return {
        title: 'Server access blocked',
        detail: `A security verification failed for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'server-status-unavailable':
      return {
        title: 'Server status unavailable',
        detail: `Cannot verify server status for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'verification-required':
      return {
        title: 'Server not verified',
        detail: `${serverName} has not been verified yet. Check the server in Server settings to access ${resource}.`,
        action: 'review-server',
      };
    case 'vault-unavailable':
      return {
        title: 'Vault unavailable',
        detail: `The contents of ${store.name} could not be loaded. ${subject}.`,
        action: 'review-server',
      };
    case 'check-in-expired':
      return {
        title: 'Check-in expired',
        detail: `The session for ${serverName} has expired. Check in again to restore access to ${resource}.`,
        action: 'open-server',
      };
    case 'check-in-unavailable':
      return {
        title: 'Check-in unavailable',
        detail: `No active session is available for ${serverName}. ${subject}.`,
        action: 'open-server',
      };
    case 'schema-incompatible':
      return {
        title: 'Vault schema incompatible',
        detail: `${subject}. This data is incompatible with your current version of FOKS. Please update FOKS or contact your administrator.`,
        action: 'review-server',
      };
    case 'import-verification-required':
      return {
        title: 'Verification required',
        detail: `${subject} until the imported profile is verified online.`,
        action: 'review-server',
      };
    case 'agent-unavailable':
      return {
        title: 'Local service unavailable',
        detail: subject,
        action: 'review-server',
      };
    case 'setup-incomplete':
      return {
        title: 'Group setup incomplete',
        detail: 'Items and members are unavailable until setup is finished.',
        action: 'finish-setup',
      };
  }
}

export interface StoreAccessTakeoverProps {
  snapshot: AgentSnapshot;
  store: Store;
  onOpenServer: (profile: string) => void;
  onFinishSetup: () => void;
  headerAction?: ReactNode;
  noHeader?: boolean;
  /**
   * `band` draws the message as a full-width alert with its one action at the
   * right end, on the alert's centre line — the group page's treatment.
   */
  variant?: 'notice' | 'band';
}

export function StoreAccessTakeover({
  snapshot,
  store,
  onOpenServer,
  onFinishSetup,
  headerAction,
  noHeader = false,
  variant = 'notice',
}: StoreAccessTakeoverProps): ReactNode {
  const state = storeDescriptionState(snapshot, store);
  if (state === 'normal') return null;
  const server = serverOf(snapshot, store.id);
  const copy = accessCopy(state, store, server?.name ?? store.server);
  const action =
    copy.action === 'finish-setup' ? (
      <Button variant="primary" onClick={onFinishSetup}>
        Finish setup
      </Button>
    ) : (
      <Button onClick={() => onOpenServer(server?.id ?? store.server)}>
        {copy.action === 'open-server' ? 'View server' : 'Server settings'}
      </Button>
    );

  return (
    <>
      {noHeader ? null : (
        <PageHeader
          title={store.name}
          subtitle={storeHeadingDescription(snapshot, store)}
          action={
            <>
              {state === 'setup-incomplete' ? null : action}
              {headerAction}
            </>
          }
        />
      )}
      <div className="body">
        {variant === 'band' ? (
          // The takeover replaces the page's content, so what it says is a
          // heading of that page; the band draws the same words as its lead-in,
          // which is a phrase in a sentence rather than a heading.
          <>
            <h2 className="offscreen">{copy.title}</h2>
            <Band
              severity={state === 'setup-incomplete' ? 'warn' : 'crit'}
              label={copy.title}
              action={action}
            >
              {copy.detail}
            </Band>
          </>
        ) : (
          <Notice
            severity={state === 'setup-incomplete' ? 'warn' : 'crit'}
            title={copy.title}
            actions={action}
          >
            <p>{copy.detail}</p>
          </Notice>
        )}
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

/** Aggregates store access problems by server for warning banners. */
export function storeAccessBands(snapshot: AgentSnapshot): StoreAccessBand[] {
  const buckets = new Map<string, { state: AccessProblem; stores: Store[] }>();
  for (const store of snapshot.stores) {
    const state = storeDescriptionState(snapshot, store);
    if (state === 'normal' || state === 'setup-incomplete') continue;
    const key = `${state}:${store.server}`;
    const bucket = buckets.get(key) ?? { state, stores: [] };
    bucket.stores.push(store);
    buckets.set(key, bucket);
  }

  return [...buckets.entries()].map(([key, { state, stores }]) => {
    const names = joinNames(stores);
    const verb = stores.length === 1 ? 'is' : 'are';
    const serverName =
      serverOf(snapshot, stores[0].id)?.name ?? stores[0].server;
    const text =
      state === 'verification-failed'
        ? `${names} ${verb} unavailable because security verification failed for ${serverName}.`
        : state === 'server-status-unavailable'
          ? `${names} ${verb} unavailable because server status could not be verified for ${serverName}.`
          : state === 'check-in-expired'
            ? `${names} ${verb} unavailable because the signed check-in for ${serverName} expired.`
            : state === 'vault-unavailable'
              ? `${names} ${verb} unavailable because the latest item list could not be loaded.`
              : state === 'check-in-unavailable'
                ? `${names} ${verb} unavailable because no signed check-in is available.`
                : state === 'schema-incompatible'
                  ? `${names} ${verb} unavailable because its schema is incompatible.`
                  : state === 'import-verification-required'
                    ? `${names} ${verb} unavailable until import verification completes.`
                    : state === 'agent-unavailable'
                      ? `${names} ${verb} unavailable while the local service is stopped.`
                      : `${names} ${verb} unavailable because ${serverName} has not been checked.`;
    return { key, text };
  });
}
