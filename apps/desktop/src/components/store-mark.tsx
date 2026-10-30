/**
 * Store and team marks: an initial rendered in a hue over a matching tint
 * (`.store-mark` in shell.css). Each store uses the same mark across the
 * sidebar, list rows, details header, and sheets for consistent visual
 * identification.
 *
 * Account vaults use the username's initial and avatar hue; teams use the team
 * name's initial and the hue assigned by `storeHues`. Inactive teams use the
 * muted `--faint` color.
 */

import { createContext } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import {
  storeHues,
  storeNavigationOrder,
  hue as nameHue,
  usernameOf,
} from '../model';
import type { AgentSnapshot, Store } from '../model';

/** `sm` 16px (sidebar, breadcrumbs), `md` 24px (rows), `sheet` 36px, `lg` 48px. */
export type MarkSize = 'sm' | 'md' | 'sheet' | 'lg';

export interface MarkProps {
  /** A `--mark-*` token reference from `HUES`, or any CSS color value. */
  hue: string;
  /** One or two initial characters. */
  children: ReactNode;
  size?: MarkSize;
  /** Uses `--faint` text color instead of the hue for inactive stores. */
  off?: boolean;
  className?: string;
}

export function Mark({
  hue,
  children,
  size = 'sm',
  off = false,
  className,
}: MarkProps): ReactNode {
  return (
    <span
      className={['store-mark', size, off ? 'off' : '', className ?? '']
        .filter(Boolean)
        .join(' ')}
      style={{ '--h': hue } as CSSProperties}
      // Decorative initial adjacent to the visible store name.
      aria-hidden="true"
    >
      {children}
    </span>
  );
}

/**
 * Context mapping store IDs to mark hues for components without direct access
 * to the snapshot (`GroupMark`). The shell populates this from `storeMarkFace`
 * so each team renders with a consistent color across views.
 */
export const StoreHueContext = createContext<ReadonlyMap<
  string,
  string
> | null>(null);

/** Builds a map of store ID to mark hue for all stores in `snapshot`. */
export function storeMarkHues(snapshot: AgentSnapshot): Map<string, string> {
  return new Map(
    snapshot.stores.map((store) => [
      store.id,
      storeMarkFace(snapshot, store).hue,
    ]),
  );
}

/** Resolves the mark hue, initial label, and inactive state for `store`. */
export function storeMarkFace(
  snapshot: AgentSnapshot,
  store: Store,
): { hue: string; label: string; off: boolean } {
  if (store.kind === 'account') {
    const name = usernameOf(snapshot, store) ?? store.account;
    return {
      hue: nameHue(name),
      label: name.slice(0, 1).toUpperCase(),
      off: false,
    };
  }
  return {
    hue:
      storeHues(storeNavigationOrder(snapshot)).get(store.id) ??
      nameHue(store.name),
    label: store.name.slice(0, 1).toUpperCase(),
    off: store.kind === 'team' && store.active === false,
  };
}

export function StoreMark({
  snapshot,
  store,
  size = 'sm',
  className,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  size?: MarkSize;
  className?: string;
}): ReactNode {
  const face = storeMarkFace(snapshot, store);
  return (
    <Mark hue={face.hue} size={size} off={face.off} className={className}>
      {face.label}
    </Mark>
  );
}
