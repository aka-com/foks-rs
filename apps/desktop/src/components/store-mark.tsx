/**
 * Store and team marks: an initial drawn in a hue over a tint of that hue
 * (`.store-mark` in shell.css). One mark for a store wherever it is listed — the
 * sidebar, list rows, the details hero, sheets — so the reader can match them
 * by colour as well as by name.
 *
 * An account vault is marked with its username's initial and hue, the same
 * hue the account's avatar uses; a team with its own name's initial and the
 * hue `storeHues` assigns it. An inactive team draws the faint ink.
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

/** `sm` 16px (sidebar, crumbs), `md` 24px (rows), `sheet` 36px, `lg` 48px. */
export type MarkSize = 'sm' | 'md' | 'sheet' | 'lg';

export interface MarkProps {
  /** A `--mark-*` reference from `HUES`, or any colour. */
  hue: string;
  /** One or two letters. */
  children: ReactNode;
  size?: MarkSize;
  /** Draws the faint ink instead of the hue, for an inactive store. */
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
      // The initial stands for the name drawn beside it.
      aria-hidden="true"
    >
      {children}
    </span>
  );
}

/**
 * The hue each store is marked with on screen, for marks drawn by code that
 * has no snapshot to hand (`GroupMark`). The shell provides it from the same
 * `storeMarkFace` the sidebar uses, so a team has one colour everywhere.
 */
export const StoreHueContext = createContext<ReadonlyMap<
  string,
  string
> | null>(null);

/** Every store's mark hue, keyed by store id. */
export function storeMarkHues(snapshot: AgentSnapshot): Map<string, string> {
  return new Map(
    snapshot.stores.map((store) => [
      store.id,
      storeMarkFace(snapshot, store).hue,
    ]),
  );
}

/** The hue and initial a store is marked with. */
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
