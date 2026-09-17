/**
 * Toolbar displayed above the item list, inside the folder browser's list
 * pane.
 *
 * Provides controls for filtering by kind, sorting items, and creating a new
 * one. The tree is permanent navigation now, so there is no view-mode toggle
 * here, and the inspector is a permanent column rather than one this toolbar
 * opens and closes.
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
import type { FoksIconName } from '../icons';
import type { KindFilter, SortKey } from '../location';

const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  name: 'By name',
  kind: 'By kind',
  group: 'Grouped',
};

const SORT_ICONS: Readonly<Record<SortKey, FoksIconName>> = {
  name: 'sortName',
  kind: 'sortKind',
  group: 'sortGroup',
};

export interface ToolbarProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  kind: KindFilter;
  onKind: (kind: KindFilter) => void;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
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
  sort,
  onSort,
  onSettings,
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
      {/* Sort selection menu trigger. */}
      <MenuButton
        label={<Icon name={SORT_ICONS[sort]} />}
        menuLabel="Item order"
        align="start"
        className="sortwrap"
        title={SORT_LABELS[sort]}
        aria-label={SORT_LABELS[sort]}
      >
        {(close) => (
          <>
            {(Object.keys(SORT_LABELS) as SortKey[]).map((key) => (
              <button
                key={key}
                type="button"
                role="menuitemradio"
                className={sort === key ? 'on' : ''}
                aria-checked={sort === key}
                onClick={() => {
                  onSort(key);
                  close();
                }}
              >
                <Icon name={SORT_ICONS[key]} className="mark" />
                {SORT_LABELS[key]}
              </button>
            ))}
          </>
        )}
      </MenuButton>
      <span className="spacer" />
      <NewItemButton onNew={onNew} />
      {onSettings ? (
        <Button
          variant="quiet"
          icon="gear"
          title="Team settings"
          aria-label="Team settings"
          onClick={onSettings}
        />
      ) : null}
    </div>
  );
}
