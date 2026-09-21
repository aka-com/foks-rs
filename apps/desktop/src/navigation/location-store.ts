import type { Store, StoreRef } from '../model/types';
import { chatTabLocation } from './chat-tab-memory';
import { accountAtLocation, railTabOf, sameLocation } from './routes';
import { transition } from './transition';
import { INITIAL_STATE } from './types';
import type {
  GuardedOptions,
  GuardVerdict,
  KindFilter,
  Location,
  LocationAction,
  LocationState,
  NavigateOptions,
  NavigationGuard,
  NavigationIntent,
  NavigationPrompter,
  RailTab,
  Scene,
  Selection,
  SortKey,
  SortDirection,
} from './types';

/* ----------------------------------------------------------------- store -- */

/**
 * External navigation store satisfying React's `useSyncExternalStore` contract.
 */
export class LocationStore {
  private current: LocationState;
  private stores: readonly Store[] = [];
  private actingAccount?: StoreRef;
  private hasInventory = false;
  private readonly tabs = new Map<RailTab, LocationState>();
  private readonly history: LocationState[] = [];
  private readonly future: LocationState[] = [];

  backTarget(): Location | null {
    return this.current.location.kind === 'first-run'
      ? null
      : (this.history.at(-1)?.location ?? null);
  }

  forwardTarget(): Location | null {
    return this.current.location.kind === 'first-run'
      ? null
      : (this.future.at(-1)?.location ?? null);
  }

  back(options: GuardedOptions = {}): void {
    this.travel(this.history, this.future, options);
  }

  forward(options: GuardedOptions = {}): void {
    this.travel(this.future, this.history, options);
  }

  private travel(
    from: LocationState[],
    to: LocationState[],
    options: GuardedOptions,
  ): void {
    const target = from.at(-1);
    if (!target || this.current.location.kind === 'first-run') return;
    const origin = this.current;
    this.guarded(
      { kind: 'navigate', location: target.location },
      () => {
        // A delayed confirmation cannot replay history after another move.
        if (
          from.at(-1) !== target ||
          !sameLocation(this.current.location, origin.location) ||
          this.current.folder !== origin.folder
        )
          return;
        from.pop();
        to.push({ ...this.current, sheet: undefined });
        this.accountLocation(target.location);
        this.publish({ ...target, sheet: undefined }, false);
      },
      options,
    );
  }

  clearTabMemory(): void {
    this.history.length = 0;
    this.future.length = 0;
    this.pendingPrompt = null;
    this.publish({ ...this.current, sheet: undefined }, false);
    this.tabs.clear();
  }

  private readonly sheetRestoration = new Map<symbol, boolean>();
  setSheetRestorable(id: symbol, restorable: boolean | undefined): void {
    if (restorable === undefined) this.sheetRestoration.delete(id);
    else this.sheetRestoration.set(id, restorable);
  }

  setSheetField(key: string, value: unknown): void {
    if (Object.is(this.current.sheet?.[key], value)) return;
    this.publish({
      ...this.current,
      sheet: { ...this.current.sheet, [key]: value },
    });
  }

  clearSheet(): void {
    if (this.current.sheet) this.publish({ ...this.current, sheet: undefined });
  }

  /**
   * Where a rail tab goes, and the state it resumes there, without moving.
   * `null` when the tab already owns the current page. Pure: the acting
   * account is recorded by the navigation itself, not by working out its
   * destination, so a guard can be asked before anything changes.
   */
  private tabTarget(
    tab: RailTab,
  ): { location: Location; saved?: LocationState } | null {
    if (railTabOf(this.current.location) === tab) return null;
    const defaults: Record<RailTab, Location> = {
      chat: chatTabLocation(),
      files: { kind: 'files' },
      teams: { kind: 'teams' },
      devices: { kind: 'devices' },
      settings: { kind: 'settings' },
    };
    let saved = this.tabs.get(tab);
    let location = saved?.location ?? defaults[tab];
    // A Teams sheet intent is spent where it was handed over. The tab resumes
    // the list, never the sheet another screen once asked for.
    if (location.kind === 'teams' && location.open)
      location = {
        kind: 'teams',
        ...(location.store ? { store: location.store } : {}),
      };
    const target = 'ref' in location ? location.ref : undefined;
    if (
      this.hasInventory &&
      target &&
      !this.stores.some((store) => store.id === target)
    ) {
      saved = undefined;
      location = tab === 'chat' ? { kind: 'chat' } : defaults[tab];
    }
    if (
      location.kind === 'teams' ||
      location.kind === 'devices' ||
      location.kind === 'settings'
    ) {
      const account = this.getAccount();
      // A canonical tab address has no account of its own. Only an explicit
      // address for another account invalidates saved sheet or detail state.
      const changed =
        location.store !== undefined && location.store !== account;
      if (changed && saved) saved = { ...saved, sheet: undefined };
      location = { ...location, store: account };
      if (changed && location.kind === 'devices') delete location.device;
      if (changed && location.kind === 'settings') delete location.profile;
    }
    return {
      location: this.resolvedLocation(location),
      ...(saved ? { saved } : {}),
    };
  }

  /** Rail tabs resume their last page; explicit home links still open roots. */
  navigateTab(tab: RailTab, options: GuardedOptions = {}): void {
    const target = this.tabTarget(tab);
    if (!target) return;
    const apply = (): void => {
      const location = this.accountLocation(target.location);
      const next = transition(this.current, { type: 'navigate', location });
      this.publish(
        target.saved
          ? { ...target.saved, location, selection: null, details: false }
          : { ...next, query: '', details: false },
      );
    };
    this.guarded(
      { kind: 'navigate', location: target.location, tab: true },
      apply,
      options,
    );
  }

  /** Inventory is refreshed by the shell; a removed account is never reused. */
  setAccountStores(stores: readonly Store[]): void {
    this.hasInventory = true;
    this.stores = stores;
    this.actingAccount = accountAtLocation(
      stores,
      this.current.location,
      this.actingAccount,
    )?.id;
  }

  getAccount(): StoreRef | undefined {
    return accountAtLocation(
      this.stores,
      this.current.location,
      this.actingAccount,
    )?.id;
  }

  private readonly listeners = new Set<() => void>();

  constructor(initial: LocationState = INITIAL_STATE) {
    this.current = initial;
  }

  readonly getSnapshot = (): LocationState => this.current;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Apply an action. Publishes only when the state actually changed. */
  dispatch(action: LocationAction): LocationState {
    const next = transition(this.current, action);
    return this.publish(next, !(action.type === 'navigate' && action.replace));
  }

  private publish(next: LocationState, recordHistory = true): LocationState {
    if (next === this.current) return next;
    if (
      recordHistory &&
      (!sameLocation(next.location, this.current.location) ||
        next.folder !== this.current.folder) &&
      this.current.location.kind !== 'first-run' &&
      next.location.kind !== 'first-run'
    ) {
      this.future.length = 0;
      this.history.push({ ...this.current, sheet: undefined });
      if (this.history.length > 100) this.history.shift();
    }
    const tab = railTabOf(this.current.location);
    if (tab) this.tabs.set(tab, this.current);
    this.current = next;
    for (const listener of this.listeners) listener();
    return next;
  }

  /**
   * The address a navigation lands on, with the account it is read through
   * filled in. Pure — `accountLocation` is the same resolution, and also
   * records that account as the acting one.
   */
  private resolvedLocation(location: Location): Location {
    const account = accountAtLocation(this.stores, location, this.getAccount());
    if (
      account &&
      (location.kind === 'devices' ||
        location.kind === 'settings' ||
        location.kind === 'teams') &&
      !location.store
    )
      return { ...location, store: account.id };
    return location;
  }

  private accountLocation(location: Location): Location {
    const account = accountAtLocation(this.stores, location, this.getAccount());
    if (account) this.actingAccount = account.id;
    return this.resolvedLocation(location);
  }

  /* ------------------------------------------------------------- guards -- */

  private readonly guards: NavigationGuard[] = [];
  private prompter: NavigationPrompter | null = null;
  private refusalHandler: ((reason: string) => void) | null = null;
  /**
   * The prompt on screen, if any. Every guarded operation replaces it: the
   * newer intent is the one the reader is asking for, so the older prompt's
   * answer, whenever it arrives, is discarded.
   */
  private pendingPrompt: object | null = null;

  /**
   * Registers a guard, returning the function that takes it off again. Guards
   * are asked in registration order and the first non-null verdict decides.
   */
  registerGuard(guard: NavigationGuard): () => void {
    this.guards.push(guard);
    return () => {
      const at = this.guards.indexOf(guard);
      if (at >= 0) this.guards.splice(at, 1);
    };
  }

  /**
   * What the guards say about an intent, without acting on it.
   */
  navigationVerdict(intent: NavigationIntent): GuardVerdict {
    // A refusal terminates validation immediately. Prompts remain provisional
    // until every guard has been evaluated because a subsequent guard may
    // still reject navigation.
    let prompt: GuardVerdict = null;
    for (const guard of [...this.guards]) {
      const verdict = guard(intent);
      if (verdict?.verdict === 'refuse') return verdict;
      if (verdict && !prompt) prompt = verdict;
    }
    return prompt;
  }

  /** The dialog a `prompt` verdict is put to the reader through. */
  setPrompter(prompter: NavigationPrompter | null): void {
    this.prompter = prompter;
  }

  /** Where a `refuse` verdict's reason is shown. */
  setRefusalHandler(handler: ((reason: string) => void) | null): void {
    this.refusalHandler = handler;
  }

  /**
   * Runs the guards over `intent` and applies `apply` once they are satisfied:
   * now for an allowed or forced move, and when the prompt is confirmed for a
   * prompted one. The caller's signature stays synchronous either way.
   */
  private guarded(
    intent: NavigationIntent,
    applyRequested: () => void,
    options: GuardedOptions,
  ): void {
    const apply = (): void => {
      if ([...this.sheetRestoration.values()].includes(false))
        this.clearSheet();
      applyRequested();
    };
    const verdict = options.force ? null : this.navigationVerdict(intent);
    // A refusal changes nothing, including an open prompt about another move.
    if (verdict?.verdict === 'refuse') {
      this.refusalHandler?.(verdict.reason);
      return;
    }
    // Anything else supersedes that prompt: its answer no longer applies.
    this.pendingPrompt = null;
    if (!verdict) {
      apply();
      return;
    }
    const prompter = this.prompter;
    if (!prompter) {
      apply();
      return;
    }
    const token = {};
    this.pendingPrompt = token;
    void prompter(verdict).then(
      (confirmed) => {
        if (this.pendingPrompt !== token) return;
        this.pendingPrompt = null;
        if (!confirmed) return;
        verdict.onConfirm?.();
        this.clearSheet();
        apply();
      },
      () => {
        if (this.pendingPrompt === token) this.pendingPrompt = null;
      },
    );
  }

  navigate(location: Location, options: NavigateOptions = {}): void {
    const apply = (): void => {
      this.dispatch({
        type: 'navigate',
        location: this.accountLocation(location),
        ...(options.replace ? { replace: true } : {}),
      });
    };
    this.guarded(
      { kind: 'navigate', location: this.resolvedLocation(location) },
      apply,
      options,
    );
  }

  /**
   * Opens `location` with `selection` showing on it, as one guarded move. The
   * ⌘K palette's item results go through this: a guard that holds the
   * navigation back must hold the selection with it, or the reader would be
   * left on the page they were on with another page's item in the details
   * panel. The guards see the navigation; the selection arrives with it.
   */
  navigateAndSelect(
    location: Location,
    selection: Selection,
    options: NavigateOptions = {},
  ): void {
    const apply = (): void => {
      this.dispatch({
        type: 'navigate',
        location: this.accountLocation(location),
        ...(options.replace ? { replace: true } : {}),
      });
      this.dispatch({ type: 'select', selection });
    };
    this.guarded(
      { kind: 'navigate', location: this.resolvedLocation(location) },
      apply,
      options,
    );
  }

  select(
    selection: Selection,
    options: GuardedOptions & { closeDetails?: boolean } = {},
  ): void {
    this.guarded(
      { kind: 'select', selection },
      () => {
        this.dispatch({
          type: 'select',
          selection,
          closeDetails: options.closeDetails,
        });
      },
      options,
    );
  }

  search(query: string): void {
    this.dispatch({ type: 'search', query });
  }

  setDetails(open: boolean): void {
    this.dispatch({ type: 'details', open });
  }

  setKind(kind: KindFilter): void {
    this.dispatch({ type: 'kind', kind });
  }

  setSort(sort: SortKey, direction: SortDirection = 'asc'): void {
    this.dispatch({ type: 'sort', sort, direction });
  }

  setFolder(folder: string): void {
    this.dispatch({ type: 'folder', folder });
  }

  toggleFolder(folder: string): void {
    this.dispatch({ type: 'toggle-folder', folder });
  }
}

/** Initializes a LocationStore populated with state from a decoded Scene. */
export function storeAtScene(scene: Scene): LocationStore {
  return new LocationStore({
    ...INITIAL_STATE,
    location: scene.location,
    selection: scene.selection,
    details: scene.selection !== null,
    view: scene.view,
    kind: scene.kind,
    sort: scene.sort,
    sortDirection: scene.sortDirection,
    folder: scene.folder,
    closedFolders: scene.closedFolders,
  });
}
