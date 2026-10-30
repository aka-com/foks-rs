/**
 * View options menu in the Files list bar: switches between List and Cards,
 * filters the open store or folder by item kind, and selects the sort key and
 * direction. Renders options as `menuitemradio` items with a leading checkmark
 * and keeps focus on the menu trigger after selection.
 */

import type { ReactNode } from 'react';
import { Icon, MenuButton } from '../components';
import { setFilesView, useFilesView } from '../files-view-pref';
import type { FilesView } from '../files-view-pref';
import type { KindFilter, SortDirection, SortKey } from '../location';

function Choice({
  checked,
  onSelect,
  children,
}: {
  checked: boolean;
  onSelect: () => void;
  children: ReactNode;
}): ReactNode {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={checked}
      className="choice"
      onClick={onSelect}
    >
      <span className="tick" aria-hidden="true">
        {checked ? <Icon name="check" /> : null}
      </span>
      {children}
    </button>
  );
}

export function FilesViewMenu({
  sort,
  direction,
  onSort,
  locationSort,
  kind,
  onKind,
  disabled = false,
}: {
  /** Active item kind filter for the open store or folder. */
  kind: KindFilter;
  onKind: (kind: KindFilter) => void;
  sort: SortKey;
  direction: SortDirection;
  onSort: (sort: SortKey, direction: SortDirection) => void;
  /** True when the list spans multiple stores, enabling the Location sort option. */
  locationSort: boolean;
  disabled?: boolean;
}): ReactNode {
  const view = useFilesView();
  // Fall back to sorting by name when scoped to a single store.
  const shown = !locationSort && sort === 'group' ? 'name' : sort;
  const views: readonly [FilesView, string][] = [
    ['list', 'As List'],
    ['grid', 'As Cards'],
  ];
  const kinds: readonly [KindFilter, string][] = [
    ['All', 'All Items'],
    ['Password', 'Passwords'],
    ['Document', 'Documents'],
  ];
  const keys: [SortKey, string][] = [
    ['name', 'Name'],
    ['kind', 'Kind'],
  ];
  if (locationSort) keys.push(['group', 'Location']);
  return (
    <MenuButton
      label=""
      aria-label="View options"
      title="View options"
      variant="quiet"
      icon="ellipsis"
      trailingIcon={null}
      menuLabel="View options"
      align="end"
      className="viewwrap"
      disabled={disabled}
    >
      {() => (
        <>
          {views.map(([id, label]) => (
            <Choice
              key={id}
              checked={view === id}
              onSelect={() => setFilesView(id)}
            >
              {label}
            </Choice>
          ))}
          <div className="menu-separator" role="separator" />
          <div className="mh" aria-hidden="true">
            Show
          </div>
          {kinds.map(([id, label]) => (
            <Choice key={id} checked={kind === id} onSelect={() => onKind(id)}>
              {label}
            </Choice>
          ))}
          <div className="menu-separator" role="separator" />
          <div className="mh" aria-hidden="true">
            Sort By
          </div>
          {keys.map(([id, label]) => (
            <Choice
              key={id}
              checked={shown === id}
              onSelect={() => onSort(id, direction)}
            >
              {label}
            </Choice>
          ))}
          <div className="menu-separator" role="separator" />
          <Choice
            checked={direction === 'asc'}
            onSelect={() => onSort(shown, 'asc')}
          >
            Ascending
          </Choice>
          <Choice
            checked={direction === 'desc'}
            onSelect={() => onSort(shown, 'desc')}
          >
            Descending
          </Choice>
        </>
      )}
    </MenuButton>
  );
}
