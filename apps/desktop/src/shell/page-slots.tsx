/**
 * Places in the shell's chrome that a page fills: the trailing end of the
 * panel's first bar, for a page's own commands (Teams: Find Teams, Join a
 * Team…, Create a Team…), and the details column, for a page that is a list
 * beside its details (Activity). The shell owns the elements and a screen
 * portals into them, so the commands keep the screen's own state and
 * handlers.
 *
 * Without a shell around it (a screen rendered on its own, as the tests do)
 * each slot draws its children in place.
 */

import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * The bar's slot element. `undefined` means no shell provides one; `null`
 * means the shell's element is not mounted yet.
 */
export const BarActionsSlot = createContext<HTMLElement | null | undefined>(
  undefined,
);

/** The details column's element, with the same three states. */
export const DetailsSlot = createContext<HTMLElement | null | undefined>(
  undefined,
);

/** A page's commands, drawn at the trailing end of the panel's first bar. */
export function BarActions({ children }: { children: ReactNode }): ReactNode {
  const slot = useContext(BarActionsSlot);
  if (slot === undefined) return <div className="page-acts">{children}</div>;
  return slot ? createPortal(children, slot) : null;
}

/** A page's details column, drawn beside the panel's list column. */
export function DetailsColumn({
  children,
}: {
  children: ReactNode;
}): ReactNode {
  const slot = useContext(DetailsSlot);
  if (slot === undefined) return children;
  return slot ? createPortal(children, slot) : null;
}
