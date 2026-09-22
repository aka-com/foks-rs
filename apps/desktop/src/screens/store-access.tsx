/** Full-page error and recovery view displayed when a store cannot be accessed. */

import type { ReactNode } from 'react';
import { Band, Button, Notice } from '../components';
import {
  serverDisplayLabelForStore as displayServerName,
  canChangeItem,
  storeAvailability,
  storeOf,
  serverOf,
  storeDescriptionState,
} from '../model';
import type {
  Item,
  Store,
  StoreDescriptionState,
  AgentSnapshot,
  StoreOperation,
} from '../model';

export function itemActionProblem(
  snapshot: AgentSnapshot,
  item: Item,
  write: boolean,
  nowSeconds: number,
): string | undefined {
  const current = snapshot.items.find(
    (entry) => entry.store === item.store && entry.path === item.path,
  );
  if (!current)
    return 'This item is no longer available. Refresh the vault before continuing.';
  const store = storeOf(snapshot, current.store);
  if (!store)
    return 'This vault is unavailable. Refresh the vault before continuing.';
  const options = { nowSeconds };
  const access = storeAvailability(snapshot, store, options);
  if (!access.available)
    return accessCopy(access.reason, store, displayServerName(snapshot, store))
      .detail;
  if (write && !canChangeItem(snapshot, current, options))
    return 'You do not have permission to change this item.';
  return undefined;
}

export type AccessProblem = Exclude<StoreDescriptionState, 'normal'>;

export interface StoreAccessCopy {
  title: string;
  detail: string;
  action: 'open-server' | 'review-server' | 'finish-setup';
  actionLabel?: string;
  actionDisabled?: boolean;
}

function resourceSubject(store: Store, operation: StoreOperation): string {
  if (operation === 'vault') return `items in ${store.name}`;
  if (operation === 'chat') return `messages in ${store.name}`;
  if (operation === 'teams') return `members in ${store.name}`;
  if (operation === 'federation')
    return `shared access details for ${store.name}`;
  return store.kind === 'team'
    ? `items and members in ${store.name}`
    : `items in ${store.name}`;
}

/**
 * The one heading and sentence for a store that cannot be opened. Every place
 * that states the condition — the takeover, the incomplete-group page,
 * the Channels tab's band — reads it from here, so none of them can describe
 * the same state differently.
 */
export function accessCopy(
  state: AccessProblem,
  store: Store,
  serverName: string,
  operation: StoreOperation = 'vault',
): StoreAccessCopy {
  const resource = resourceSubject(store, operation);
  const subject = `${resource[0].toUpperCase()}${resource.slice(1)} are unavailable`;
  switch (state) {
    case 'chat-unsupported':
      return {
        title: 'Chat not supported',
        detail: `${serverName} does not advertise chat for this store.`,
        action: 'open-server',
      };
    case 'store-metadata-unavailable':
      return {
        title: 'Store information unavailable',
        detail: `The current identity and setup information for ${store.name} could not be loaded.`,
        action: 'review-server',
      };
    case 'loading':
      return {
        title: 'Loading',
        detail: `Loading access information for ${store.name}.`,
        action: 'open-server',
      };
    case 'compatibility-incompatible':
      return {
        title: 'Protocol incompatible',
        detail: `Compatibility verification does not permit access to ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'capability-unavailable':
      return {
        title: 'Operation not permitted',
        detail: `The required capability is not granted for ${serverName}. ${subject}.`,
        action: 'review-server',
      };
    case 'security-state-missing':
      return {
        title: 'Saved security state missing',
        detail: `Restore saved trust for ${serverName} before continuing. No replacement identity will be established automatically.`,
        action: 'review-server',
      };
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
        detail: `${subject} because the latest contents of ${store.name} could not be loaded.`,
        action: 'review-server',
      };
    case 'check-in-expired':
      return {
        title: 'Check-in expired',
        detail: `Compatibility verification for ${serverName} has expired. A newer signed verification is required to restore access to ${resource}.`,
        action: 'open-server',
      };
    case 'check-in-unavailable':
      return {
        title: 'Check-in unavailable',
        detail: `No signed compatibility verification is available for ${serverName}. ${subject}.`,
        action: 'open-server',
      };
    case 'schema-incompatible':
      return {
        title: 'Vault schema incompatible',
        detail: `${subject}. This data is incompatible with the installed version of FOKS. Update FOKS or contact your administrator.`,
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
        // Every other state names the condition as well as the subject; this
        // one said only the subject.
        detail: `${subject} while the local service is stopped.`,
        action: 'review-server',
      };
    case 'setup-incomplete': {
      const phase = store.kind === 'team' ? store.creation_phase : undefined;
      const verified = phase === 'remote-verified';
      const preparing = phase === 'preparing';
      const rejected = phase === 'rejected';
      return {
        title: 'Setup incomplete',
        detail: verified
          ? `${store.name} was created on ${serverName}, but key setup is incomplete on this device. Members and items are unavailable until setup finishes.`
          : preparing
            ? `Creation of ${store.name} has not been submitted to ${serverName}. Continue with the saved team identity and keys.`
            : rejected
              ? `Creation of ${store.name} was rejected. Its saved identity and keys are retained on this device.`
              : `Creation of ${store.name} must be checked on ${serverName}. Its saved identity and keys will be used to verify the result before setup continues.`,
        action: 'finish-setup',
        actionLabel: verified
          ? 'Finish setup'
          : preparing
            ? 'Continue creation'
            : 'Check creation',
        actionDisabled: rejected,
      };
    }
  }
}

export interface StoreAccessTakeoverProps {
  operation?: StoreOperation;
  snapshot: AgentSnapshot;
  store: Store;
  onOpenServer: (profile: string) => void;
  onFinishSetup: () => void;
  headerAction?: ReactNode;
  /**
   * `band` draws the message as a full-width alert with its one action at the
   * right end, on the alert's center line — the group page's treatment.
   */
  variant?: 'notice' | 'band';
}

export function StoreAccessTakeover({
  operation = 'vault',
  snapshot,
  store,
  onOpenServer,
  onFinishSetup,
  headerAction,
  variant = 'notice',
}: StoreAccessTakeoverProps): ReactNode {
  const state = storeDescriptionState(snapshot, store, { operation });
  if (state === 'normal') return null;
  const server = serverOf(snapshot, store.id);
  const copy = accessCopy(
    state,
    store,
    displayServerName(snapshot, store),
    operation,
  );
  const action =
    copy.action === 'finish-setup' ? (
      <Button
        variant="primary"
        onClick={onFinishSetup}
        disabled={copy.actionDisabled}
      >
        {copy.actionLabel ?? 'Finish setup'}
      </Button>
    ) : (
      <Button onClick={() => onOpenServer(server?.profileName ?? store.server)}>
        {copy.action === 'open-server' ? 'View server' : 'Server settings'}
      </Button>
    );

  return (
    <>
      <div className="body">
        {variant === 'band' ? (
          // The takeover replaces the page's content, so what it says is a
          // heading of that page; the band draws the same words as its lead-in,
          // which is a phrase in a sentence rather than a heading.
          <>
            <h2 className="offscreen">{copy.title}</h2>
            <Band
              severity={
                state === 'loading'
                  ? 'info'
                  : state === 'setup-incomplete'
                    ? 'warn'
                    : 'crit'
              }
              label={copy.title}
              action={
                <>
                  {action}
                  {headerAction}
                </>
              }
            >
              {copy.detail}
            </Band>
          </>
        ) : (
          // The window header's crumb names the store; the notice says what
          // stands between the reader and its items, with every action at its
          // right.
          <Notice
            severity={
              state === 'loading'
                ? 'info'
                : state === 'setup-incomplete'
                  ? 'warn'
                  : 'crit'
            }
            title={copy.title}
            actions={
              <>
                {action}
                {headerAction}
              </>
            }
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
    if (
      state === 'normal' ||
      state === 'setup-incomplete' ||
      state === 'loading'
    )
      continue;
    const key = `${state}:${store.server}`;
    const bucket = buckets.get(key) ?? { state, stores: [] };
    bucket.stores.push(store);
    buckets.set(key, bucket);
  }

  return [...buckets.entries()].map(([key, { state, stores }]) => {
    const names = joinNames(stores);
    const verb = stores.length === 1 ? 'is' : 'are';
    const serverName = displayServerName(snapshot, stores[0]);
    const text =
      state === 'compatibility-incompatible'
        ? `${names} ${verb} unavailable because compatibility verification does not permit access to ${serverName}.`
        : state === 'capability-unavailable'
          ? `${names} ${verb} unavailable because the required capability is not granted on ${serverName}.`
          : state === 'verification-failed'
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
