import type { KvRoleInput } from '../bridge';
import type { Store } from '../model';

/** Group items default to team-readable content that only admins can change. */
export const DEFAULT_READ_ROLE: KvRoleInput = 'Member:0';
export const DEFAULT_WRITE_ROLE: KvRoleInput = 'Admin';

/**
 * The roles a folder created from the desktop takes: the new-item defaults in
 * a team, and none in a personal vault, where the host applies its own.
 */
export function newFolderRoles(
  store: Store,
): { readRole: KvRoleInput; writeRole: KvRoleInput } | Record<string, never> {
  return store.kind === 'team'
    ? { readRole: DEFAULT_READ_ROLE, writeRole: DEFAULT_WRITE_ROLE }
    : {};
}
