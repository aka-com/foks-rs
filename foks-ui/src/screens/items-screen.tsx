/**
 * The item page — `01-vault.html`'s `renderMain()`.
 *
 * Header, toolbar, then one of five bodies: the lapsed-check-in notice, the
 * inactive-group notice, the list, the cards, or an empty state. Which one is
 * a function of the world and the navigation state; nothing here is a mode
 * someone has to remember to leave.
 *
 * Two orderings, deliberately different, both the mock's: the sidebar lists
 * stores in fixture order, and the cards view sections them in
 * `storeDisplayOrder`.
 */

import { Fragment, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { virtualListWindow } from '/kit/virtual-list';
import {
  Band,
  Button,
  Chip,
  Icon,
  KindGlyph,
  KindIcon,
  Stack,
} from '../components';
import type { FilterKind } from '../components';
import type { FoksIconName } from '../icons';
import { PageHeader, headerFor } from '../shell/page-header';
import { NewItemButton, Toolbar } from '../shell/toolbar';
import {
  KINDS,
  canChangeItem,
  catalog,
  fmtSize,
  kindOf,
  nameOf,
  partiesOf,
  peopleGroups,
  prefixOf,
  readersOf,
  serverOf,
  storeDescriptionState,
  storeDisplayOrder,
  storeOf,
  storeReadable,
} from '../model';
import type { Item, Store, World } from '../model';
import type { LocationStore, LocationState } from '../location';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemRequest } from '../bridge';
import type { NewKind } from './write-workflows';
import { readableBy, scopedItems, whereOf } from './scope';
import { StoreAccessTakeover, storeAccessBands } from './store-access';

/** The shared virtual-list models rows, not a wrapping masonry/grid layout. */
const GRID_CAP = 200;

/* --------------------------------------------------------------- pieces -- */

function PathChip({ path }: { path: string }): ReactNode {
  const prefix = prefixOf(path);
  return prefix ? <span className="pchip">{prefix}</span> : null;
}

interface RowProps {
  world: World;
  item: Item;
  selected: boolean;
  /** The full path is shown beside the store while a search is running. */
  searching: boolean;
  onSelect: () => void;
}

interface ItemActionProps {
  item: Item;
  onReveal: () => void;
  onCopyValue: () => void;
  onCopyPath: () => void;
  onDownload: () => void;
  onOpen: () => void;
  onRemove: () => void;
  removeDisabled: boolean;
}

function ItemActions({
  item,
  onReveal,
  onCopyValue,
  onCopyPath,
  onDownload,
  onOpen,
  onRemove,
  removeDisabled,
}: ItemActionProps): ReactNode {
  const kind = kindOf(item);
  const action = (
    label: string,
    icon: FoksIconName,
    run: () => void,
    danger = false,
    disabled = false,
  ) => (
    <button
      type="button"
      className={danger ? 'danger' : undefined}
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        run();
      }}
    >
      <Icon name={icon} />
    </button>
  );
  return (
    <span className="acts">
      {kind === 'Password' || kind === 'Resource' ? (
        <>
          {action('Show', 'eye', onReveal)}
          {action(
            kind === 'Password' ? 'Copy password' : 'Copy value',
            'copy',
            onCopyValue,
          )}
        </>
      ) : kind === 'File' ? (
        action('Download', 'download', onDownload)
      ) : (
        action('Open target', 'arrow', onOpen)
      )}
      {action('Copy path', 'path', onCopyPath)}
      {action(
        removeDisabled
          ? 'Your current access does not allow removing this item'
          : `Remove version ${item.version} exactly`,
        'trash',
        onRemove,
        true,
        removeDisabled,
      )}
    </span>
  );
}

function Row({
  world,
  item,
  selected,
  searching,
  onSelect,
}: RowProps): ReactNode {
  const readers = readableBy(world, item);
  const readerCount = readersOf(world, item)?.length ?? 1;
  return (
    <div
      className={selected ? 'row sel' : 'row'}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <KindIcon kind={kindOf(item) as FilterKind} />
      <span className="name">
        <span className="tt">
          <span>{nameOf(item.path)}</span>
          <PathChip path={item.path} />
        </span>
        <small>
          {whereOf(world, item)}
          {searching ? (
            <>
              {' '}
              · <code>{item.path}</code>
            </>
          ) : null}
        </small>
      </span>
      <span className="n server">{serverOf(world, item.store)?.name}</span>
      <span>
        <Chip title={readers.title}>{readerCount}</Chip>
      </span>
      <span className="n">{item.version}</span>
    </div>
  );
}

function Tile({
  world,
  item,
  selected,
  onSelect,
  onReveal,
  onCopyValue,
  onCopyPath,
  onDownload,
  onOpen,
  onRemove,
  removeDisabled,
}: Omit<RowProps, 'searching'> & ItemActionProps): ReactNode {
  const store = storeOf(world, item.store);
  const kind = kindOf(item);
  const parties = store?.kind === 'team' ? partiesOf(world, store.id) : [];
  const readers = readableBy(world, item);
  return (
    <div
      className={selected ? 'tile sel' : 'tile'}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <div className="glyph">
        <KindGlyph item={item} size="big" />
        {parties.length ? (
          <Stack
            parties={parties}
            size="xs"
            title={`In ${store?.name ?? ''} · readable by ${readers.label} of ${parties.length}`}
          />
        ) : null}
        <span className="qa">
          <ItemActions
            {...{
              item,
              onReveal,
              onCopyValue,
              onCopyPath,
              onDownload,
              onOpen,
              onRemove,
              removeDisabled,
            }}
          />
        </span>
      </div>
      <div className="cap">
        <div className="nm">{nameOf(item.path)}</div>
        <div className="sub">
          {kind === 'Link' ? (
            <>
              <PathChip path={item.path} /> target read when opened
            </>
          ) : kind === 'File' ? (
            <>
              <PathChip path={item.path} /> {fmtSize(item.size)}
            </>
          ) : prefixOf(item.path) ? (
            <PathChip path={item.path} />
          ) : (
            <span className="dim">at the root</span>
          )}
        </div>
      </div>
    </div>
  );
}

function TileSection({
  world,
  store,
}: {
  world: World;
  store: Store;
}): ReactNode {
  if (store.kind === 'team') {
    return (
      <div className="gsec">
        <Stack parties={partiesOf(world, store.id)} />
        <span>{store.name}</span>
        <span className="n">· {peopleGroups(partiesOf(world, store.id))}</span>
      </div>
    );
  }
  return (
    <div className="gsec">
      {store.name}
      <span className="n">· {serverOf(world, store.id)?.name}</span>
    </div>
  );
}

/* --------------------------------------------------------------- screen -- */

export interface ItemsScreenProps {
  world: World;
  bridge: Bridge;
  state: LocationState;
  locations: LocationStore;
  onReveal: (item: Item) => void;
  onNew: (kind: NewKind, storeId: string) => void;
  onResume: (storeId: string) => Promise<void>;
  onRemove: (item: Item) => void;
  onSettings: (storeId: string) => void;
  onCommandError: (error: unknown, item?: Item) => void;
}

export function ItemsScreen({
  world,
  bridge,
  state,
  locations,
  onReveal,
  onNew,
  onResume,
  onRemove,
  onSettings,
  onCommandError,
}: ItemsScreenProps): ReactNode {
  const toasts = useToast();
  const bodyRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [listMetrics, setListMetrics] = useState({ top: 0, viewport: 600 });
  const { location } = state;
  const store =
    location.kind === 'store' ? storeOf(world, location.ref) : undefined;
  const header = headerFor(world, location);
  const head = (
    <PageHeader
      {...header}
      query={state.query}
      onQuery={(query) => {
        locations.search(query);
      }}
    />
  );
  const items = scopedItems(world, state);

  useLayoutEffect(() => {
    const body = bodyRef.current;
    const list = listRef.current;
    if (!body || !list) return;
    const measure = (): void => {
      setListMetrics({
        top: Math.max(
          0,
          list.getBoundingClientRect().top -
            body.getBoundingClientRect().top +
            body.scrollTop,
        ),
        viewport: body.clientHeight || 600,
      });
    };
    measure();
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(measure);
    observer?.observe(body);
    return () => observer?.disconnect();
  }, [items.length, state.view, state.kind, state.query, location.kind]);

  if (store && storeDescriptionState(world, store) !== 'normal') {
    return (
      <StoreAccessTakeover
        world={world}
        store={store}
        lead={
          store.kind === 'team' ? (
            <Stack parties={partiesOf(world, store.id)} />
          ) : undefined
        }
        onOpenServer={(profile) =>
          locations.navigate({ kind: 'settings', section: 'servers', profile })
        }
        onFinishSetup={() => void onResume(store.id)}
        headerAction={
          store.kind === 'team' ? (
            <Button
              variant="quiet"
              icon="gear"
              title="Group settings"
              aria-label="Group settings"
              onClick={() => onSettings(store.id)}
            />
          ) : undefined
        }
      />
    );
  }

  const accessBands = storeAccessBands(world);
  const kindMeta = state.kind === 'All' ? null : KINDS[state.kind];
  const requestOf = (item: Item): ItemRequest => ({
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
  const report = (error: unknown): void => {
    if (normalizeCommandError(error).code === 'agent-lost')
      onCommandError(error);
    else toasts.show(normalizeCommandError(error).message);
  };
  const copyValue = (item: Item): void => {
    void bridge
      .copyItemValue(requestOf(item))
      .then(
        () =>
          toasts.show(
            `${kindOf(item) === 'Password' ? 'Password' : 'Value'} copied`,
          ),
        report,
      );
  };
  const copyPath = (item: Item): void => {
    void bridge
      .copyItemPath(requestOf(item))
      .then(() => toasts.show(`Path copied: ${item.path}`), report);
  };
  const download = (item: Item): void => {
    locations.select({ store: item.store, path: item.path });
    void bridge
      .downloadFile(requestOf(item))
      .then(
        ({ saved }) =>
          toasts.show(
            saved
              ? `Downloaded ${nameOf(item.path)} at version ${item.version}`
              : 'Download cancelled',
          ),
        report,
      );
  };
  const openLink = (item: Item): void => {
    void bridge
      .readItem(requestOf(item))
      .then((response) => {
        if (
          response.store !== item.store ||
          response.path !== item.path ||
          response.version !== item.version
        ) {
          throw new Error(
            'The local agent returned a target for a different catalog selection.',
          );
        }
        // Resolved against the catalog, not the raw item list: the raw list
        // still holds folders, and selecting one reaches KindIcon with a kind
        // it does not draw.
        const target = catalog(world).find(
          (candidate) =>
            candidate.store === item.store && candidate.path === response.value,
        );
        if (target)
          locations.select({ store: target.store, path: target.path });
        else toasts.show(`Nothing is currently at ${response.value}`);
      }, report)
      .catch(report);
  };
  const reveal = (item: Item): void => {
    locations.select({ store: item.store, path: item.path });
    // The details panel owns the returned value and its blur/hide lifetime.
    onReveal(item);
  };

  const rowWindow = virtualListWindow({
    heights: items.map(() => 50),
    listTop: listMetrics.top,
    scrollTop,
    viewport: listMetrics.viewport,
    overscan: 3,
  });
  const visibleRows = items.slice(rowWindow.start, rowWindow.end);
  const gridItems = items.slice(0, GRID_CAP);
  const callbacks = (item: Item) => ({
    onReveal: () => reveal(item),
    onCopyValue: () => copyValue(item),
    onCopyPath: () => copyPath(item),
    onDownload: () => download(item),
    onOpen: () => openLink(item),
    onRemove: () => {
      locations.select({ store: item.store, path: item.path });
      // The selected version travels with the confirmation sheet.
      onRemove(item);
    },
    removeDisabled:
      !storeReadable(world, item.store) || !canChangeItem(world, item),
  });

  return (
    <>
      {head}
      <Toolbar
        onNew={(itemKind) =>
          onNew(
            itemKind,
            store && storeReadable(world, store.id)
              ? store.id
              : 'acct:personal',
          )
        }
        kind={state.kind}
        onKind={(kind) => {
          locations.setKind(kind);
        }}
        sort={state.sort}
        onSort={(sort) => {
          locations.setSort(sort);
        }}
        view={state.view}
        onView={(view) => {
          locations.setView(view);
        }}
        details={state.details}
        onDetails={(open) => {
          locations.setDetails(open);
        }}
        onSettings={
          store?.kind === 'team' ? () => onSettings(store.id) : undefined
        }
      />
      <div
        className="body"
        ref={bodyRef}
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      >
        {location.kind === 'all'
          ? accessBands.map((band) => <Band key={band.key}>{band.text}</Band>)
          : null}
        {state.view === 'grid' && items.length > GRID_CAP ? (
          <Band>
            Showing the first {GRID_CAP} cards. Narrow the list with search or a
            kind filter to see the rest.
          </Band>
        ) : null}

        {!items.length ? (
          state.query ? (
            <div className="empty">
              <h2>
                Nothing {store ? `in ${store.name}` : 'here'} matches “
                {state.query}”
              </h2>
              <p>
                Search covers paths and store names only, and not private
                contents of items in the vault.
              </p>
            </div>
          ) : (
            <div className="empty">
              <div className="big">
                <Icon
                  name={kindMeta ? (kindMeta.icon as FoksIconName) : 'key'}
                />
              </div>
              <h2>
                No {kindMeta ? kindMeta.plural.toLowerCase() : 'items'} here
              </h2>
              <p>{(kindMeta ?? KINDS.Password).blurb}</p>
              <NewItemButton
                onNew={(itemKind) =>
                  onNew(
                    itemKind,
                    store && storeReadable(world, store.id)
                      ? store.id
                      : 'acct:personal',
                  )
                }
              />
            </div>
          )
        ) : state.view === 'list' ? (
          <div className="list-window">
            <div className="hdr">
              <span />
              <button
                type="button"
                className={state.sort === 'name' ? 'on' : ''}
                onClick={() => {
                  locations.setSort('name');
                }}
              >
                Name{state.sort === 'name' ? ' ↓' : ''}
              </button>
              <span>Server</span>
              <span>Readable by</span>
              <button
                type="button"
                className={state.sort === 'version' ? 'on' : ''}
                onClick={() => {
                  locations.setSort('version');
                }}
              >
                Version{state.sort === 'version' ? ' ↓' : ''}
              </button>
            </div>
            <div className="virtual-rows" ref={listRef}>
              {rowWindow.padTop ? (
                <div
                  className="virtual-spacer"
                  style={{ height: rowWindow.padTop }}
                />
              ) : null}
              {visibleRows.map((item) => (
                <Row
                  key={`${item.store}|${item.path}`}
                  world={world}
                  item={item}
                  searching={Boolean(state.query)}
                  selected={
                    state.selection?.store === item.store &&
                    state.selection.path === item.path
                  }
                  onSelect={() => {
                    locations.select({ store: item.store, path: item.path });
                  }}
                />
              ))}
              {rowWindow.padBottom ? (
                <div
                  className="virtual-spacer"
                  style={{ height: rowWindow.padBottom }}
                />
              ) : null}
            </div>
          </div>
        ) : location.kind === 'all' ? (
          storeDisplayOrder(world).map((sectionStore) => {
            const id = sectionStore.id;
            const section = gridItems.filter((item) => item.store === id);
            if (!section.length || !sectionStore) return null;
            return (
              <Fragment key={id}>
                <TileSection world={world} store={sectionStore} />
                <div className="tiles">
                  {section.map((item) => (
                    <Tile
                      key={`${item.store}|${item.path}`}
                      world={world}
                      item={item}
                      selected={
                        state.selection?.store === item.store &&
                        state.selection.path === item.path
                      }
                      onSelect={() => {
                        locations.select({
                          store: item.store,
                          path: item.path,
                        });
                      }}
                      {...callbacks(item)}
                    />
                  ))}
                </div>
              </Fragment>
            );
          })
        ) : (
          <>
            <div className="gsec first">
              {kindMeta ? kindMeta.plural : 'Everything'}
              <span className="n">· {items.length}</span>
            </div>
            <div className="tiles">
              {gridItems.map((item) => (
                <Tile
                  key={`${item.store}|${item.path}`}
                  world={world}
                  item={item}
                  selected={
                    state.selection?.store === item.store &&
                    state.selection.path === item.path
                  }
                  onSelect={() => {
                    locations.select({ store: item.store, path: item.path });
                  }}
                  {...callbacks(item)}
                />
              ))}
            </div>
          </>
        )}
      </div>
    </>
  );
}
