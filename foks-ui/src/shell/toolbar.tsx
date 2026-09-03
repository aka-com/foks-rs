/**
 * The item page's toolbar — `01-vault.html`'s `toolbar` string, as controls.
 *
 * New (one primary button over the kind menu) · the kind filter · Sort · list or
 * cards · the details toggle. The filter, the sort, the view and the panel
 * are navigation state, so each one is a deep link and survives a reload.
 *
 * New offers the four product kinds. The sheet binds an account create to its
 * fixed Owner roles or asks for both group roles explicitly.
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
      menuLabel="What to create"
      align="start"
    >
      {(close) => (
        <>
          {KIND_LIST.map((name) => (
            <button
              key={name}
              type="button"
              role="menuitem"
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
        label="Which kinds to list"
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
      <MenuButton label="Sort" menuLabel="Sort the list by" align="end">
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
        label="How to show the items"
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
