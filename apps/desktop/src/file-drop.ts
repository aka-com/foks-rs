/**
 * Routing for native file drops.
 *
 * The runtime intercepts OS drag events for the whole window, so every drop
 * zone in the app observes the same events; nothing in the payload says which
 * element the pointer was over. The registry keeps a drop unambiguous by
 * delivering it to a single zone: the foreground claim (a sheet or the details
 * panel) wins over the background claim (the vault content area), and among
 * equal priorities the most recently claimed zone wins.
 */

import { useEffect, useRef } from 'react';
import type { Bridge } from './bridge';

/** A registered drop zone. Only the active zone is called. */
export interface DropZone {
  onHover(hovering: boolean): void;
  onPaths(paths: string[]): void;
}

/** Background zones yield to any foreground zone layered over them. */
export type DropPriority = 'background' | 'foreground';

const RANK: Readonly<Record<DropPriority, number>> = {
  background: 0,
  foreground: 1,
};

/** Tracks the claimed drop zones and decides which one a drop belongs to. */
export class DropRegistry {
  private sequence = 0;
  private readonly claims = new Map<DropZone, { rank: number; at: number }>();

  /** Registers a zone; the returned function releases the claim. */
  claim(zone: DropZone, priority: DropPriority = 'foreground'): () => void {
    this.sequence += 1;
    this.claims.set(zone, { rank: RANK[priority], at: this.sequence });
    return () => {
      this.claims.delete(zone);
    };
  }

  /** The zone that native drops are routed to, or null when none is claimed. */
  active(): DropZone | null {
    let best: DropZone | null = null;
    let bestOrder = { rank: -1, at: -1 };
    for (const [zone, order] of this.claims) {
      if (
        order.rank > bestOrder.rank ||
        (order.rank === bestOrder.rank && order.at > bestOrder.at)
      ) {
        best = zone;
        bestOrder = order;
      }
    }
    return best;
  }

  isActive(zone: DropZone): boolean {
    return this.active() === zone;
  }
}

/** The registry the app's drop zones share. */
export const appDropRegistry = new DropRegistry();

export interface FileDropOptions {
  bridge: Bridge;
  /** Claims the drop while true; a released claim falls back to the next zone. */
  active: boolean;
  priority?: DropPriority;
  onHover: (hovering: boolean) => void;
  onPaths: (paths: string[]) => void;
  /** Reports a failure to subscribe to the native drop events. */
  onError?: (error: unknown) => void;
  registry?: DropRegistry;
}

/**
 * Subscribes to native file drops while `active`, delivering hover and drop
 * events only while this zone is the one the registry routes drops to.
 */
export function useFileDrop({
  bridge,
  active,
  priority = 'foreground',
  onHover,
  onPaths,
  onError,
  registry = appDropRegistry,
}: FileDropOptions): void {
  const handlers = useRef({ onHover, onPaths, onError });
  handlers.current = { onHover, onPaths, onError };
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let hoverOff: (() => void) | undefined;
    let pathsOff: (() => void) | undefined;
    const zone: DropZone = {
      onHover: (hovering) => handlers.current.onHover(hovering),
      onPaths: (paths) => handlers.current.onPaths(paths),
    };
    const release = registry.claim(zone, priority);
    const fail = (error: unknown): void => {
      if (!disposed) handlers.current.onError?.(error);
    };
    const keep = (off: () => void, set: (off: () => void) => void): void => {
      if (disposed) off();
      else set(off);
    };
    void bridge
      .onDropHover(({ hovering }) => {
        if (registry.isActive(zone)) zone.onHover(hovering);
      })
      .then((off) => {
        keep(off, (value) => (hoverOff = value));
      }, fail);
    void bridge
      .onDropPaths((paths) => {
        if (registry.isActive(zone)) zone.onPaths(paths);
      })
      .then((off) => {
        keep(off, (value) => (pathsOff = value));
      }, fail);
    return () => {
      disposed = true;
      release();
      hoverOff?.();
      pathsOff?.();
    };
  }, [active, bridge, priority, registry]);
}
