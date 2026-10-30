/**
 * Portal slots in the shell layout populated by active screens: the trailing
 * action container in the panel's primary topbar (e.g. Teams: Find Teams, Join
 * a Team…, Create a Team…) and the details column for split list-detail views
 * (Activity, Devices). The shell owns the target DOM nodes and screens portal
 * into them so actions retain access to screen state and handlers.
 *
 * When rendered outside the shell (such as in standalone component tests), each
 * slot renders its children inline.
 */

import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * Portal target element for topbar actions. `undefined` indicates no shell
 * provider is present; `null` indicates the shell container has not mounted yet.
 */
export const BarActionsSlot = createContext<HTMLElement | null | undefined>(
  undefined,
);

/** Portal target element for the details column, using the same three states. */
export const DetailsSlot = createContext<HTMLElement | null | undefined>(
  undefined,
);

/** Renders page-specific actions at the trailing edge of the panel's primary topbar. */
export function BarActions({ children }: { children: ReactNode }): ReactNode {
  const slot = useContext(BarActionsSlot);
  if (slot === undefined) return <div className="page-acts">{children}</div>;
  return slot ? createPortal(children, slot) : null;
}

/** Renders a page's details column alongside the main panel's list column. */
export function DetailsColumn({
  children,
}: {
  children: ReactNode;
}): ReactNode {
  const slot = useContext(DetailsSlot);
  if (slot === undefined) return children;
  return slot ? createPortal(children, slot) : null;
}
