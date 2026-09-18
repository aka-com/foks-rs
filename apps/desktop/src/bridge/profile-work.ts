import { scheduleProfileWork } from '../scheduling/profile-work';
import type { Bridge } from './contract';
import type { ServerStatusSnapshot } from './servers';

const pendingServerStatuses = new WeakMap<
  Bridge,
  Map<string, Promise<ServerStatusSnapshot>>
>();

/**
 * Queues profile-scoped requests sequentially to prevent concurrent
 * requests from failing with `profile-busy`.
 */
export function enqueueProfileWork<T>(
  bridge: Bridge,
  profile: string,
  work: () => Promise<T>,
): Promise<T> {
  return scheduleProfileWork(bridge, profile, work);
}

export function sharedServerStatus(
  bridge: Bridge,
  profile: string,
  fresh = false,
): Promise<ServerStatusSnapshot> {
  let statuses = pendingServerStatuses.get(bridge);
  if (!statuses) {
    statuses = new Map();
    pendingServerStatuses.set(bridge, statuses);
  }
  const key = `${profile}:${fresh ? 'fresh' : 'cached'}`;
  const active = statuses.get(key);
  if (active) return active;
  const pending = enqueueProfileWork(bridge, profile, () =>
    bridge.describeServerStatus(profile, fresh),
  );
  statuses.set(key, pending);
  const clear = (): void => {
    if (statuses.get(key) === pending) statuses.delete(key);
  };
  void pending.then(clear, clear);
  return pending;
}
