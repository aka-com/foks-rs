/**
 * Toolbar displayed above the item list, inside the folder browser's list
 * pane.
 *
 * Provides the kind filter, scoped search field, and New control. The list
 * column headers handle sorting, and the inspector is a permanent column.
 */

import type { ReactNode } from 'react';
import {
  KindIcon,
  MenuButton,
  SearchField,
  SegmentedControl,
} from '../components';
import { KINDS, KIND_LIST, kindLabel } from '../model';
import type { KindFilter } from '../location';

export interface ToolbarProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  kind: KindFilter;
  onKind: (kind: KindFilter) => void;
  query: string;
  onQuery: (query: string) => void;
  /** "Search all items" / "Search this vault" / "Search this team". */
  searchPlaceholder: string;
}

/** The primary New control — kind menu, no plus, a down chevron. */
export function NewItemButton({
  onNew,
}: {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
}): ReactNode {
  return (
    <MenuButton
      label="New"
      variant="primary"
      menuLabel="Create item"
      align="end"
    >
      {(close) => (
        <>
          {KIND_LIST.map((name) => (
            <button
              key={name}
              type="button"
              role="menuitem"
              className="kind-menu-item"
              onClick={() => {
                close();
                onNew(name);
              }}
            >
              <KindIcon kind={name} />
              {kindLabel(name)}
            </button>
          ))}
        </>
      )}
    </MenuButton>
  );
}

export function Toolbar({
  onNew,
  kind,
  onKind,
  query,
  onQuery,
  searchPlaceholder,
}: ToolbarProps): ReactNode {
  return (
    <div className="toolbar">
      <SegmentedControl<KindFilter>
        label="Filter items by kind"
        value={kind}
        onChange={onKind}
        items={[
          { id: 'All', label: 'All' },
          ...KIND_LIST.map((name) => ({
            id: name,
            label: KINDS[name].plural,
            title: KINDS[name].blurb,
          })),
        ]}
      />
      <span className="spacer" />
      {/* The ⌘K shortcut is reserved for the global palette. */}
      <SearchField
        value={query}
        onChange={onQuery}
        placeholder={searchPlaceholder}
        shortcut={false}
      />
      <NewItemButton onNew={onNew} />
    </div>
  );
}
