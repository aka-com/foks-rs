/**
 * Toolbar displayed above the item list.
 *
 * Provides controls for filtering by kind, sorting items, and toggling between
 * list and grid views.
 */

import type { ReactNode } from 'react';
import {
  Button,
  Icon,
  KindIcon,
  MenuButton,
  SegmentedControl,
} from '../components';
import { KINDS, KIND_LIST, kindLabel } from '../model';
import type { KindFilter, SortKey, ViewMode } from '../location';

const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  name: 'Sort by name',
  kind: 'Sort by kind',
  group: 'Grouped',
  version: 'Sort by version',
};

export interface ToolbarProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  kind: KindFilter;
  onKind: (kind: KindFilter) => void;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
  view: ViewMode;
  onView: (view: ViewMode) => void;
  details: boolean;
  onDetails: (open: boolean) => void;
  onSettings?: () => void;
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
      align="start"
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
  sort,
  onSort,
  view,
  onView,
  details,
  onDetails,
  onSettings,
}: ToolbarProps): ReactNode {
  return (
    <div className="toolbar">
      <NewItemButton onNew={onNew} />
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
      {/* Sort selection menu trigger. */}
      <MenuButton
        label={SORT_LABELS[sort]}
        menuLabel={`Sort by: ${SORT_LABELS[sort]}`}
        align="end"
      >
        {(close) => (
          <>
            {(Object.keys(SORT_LABELS) as SortKey[]).map((key) => (
              <button
                key={key}
                type="button"
                role="menuitem"
                className={sort === key ? 'on' : ''}
                aria-checked={sort === key}
                onClick={() => {
                  onSort(key);
                  close();
                }}
              >
                {sort === key ? <Icon name="check" /> : <span className="ic" />}
                {SORT_LABELS[key]}
              </button>
            ))}
          </>
        )}
      </MenuButton>
      <SegmentedControl<ViewMode>
        label="View display mode"
        variant="icon"
        value={view}
        onChange={onView}
        items={[
          { id: 'list', icon: 'list', title: 'List' },
          { id: 'grid', icon: 'grid', title: 'Cards' },
        ]}
      />
      <Button
        variant="quiet"
        icon="info"
        on={details}
        title="Details"
        aria-label="Details"
        onClick={() => {
          onDetails(!details);
        }}
      />
      {onSettings ? (
        <Button
          variant="quiet"
          icon="gear"
          title="Group settings"
          aria-label="Group settings"
          onClick={onSettings}
        />
      ) : null}
    </div>
  );
}
