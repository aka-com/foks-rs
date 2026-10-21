/**
 * Details panel displaying item metadata, content preview, sharing roster, and item actions.
 *
 * Its first bar is the item's location (store › folders, each a link to that
 * folder) and the More menu; under it the hero (kind tile, name, kind and
 * server, the primary actions) and form-style property rows in groups divided
 * by hairlines.
 */

import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useToast } from '/kit/toasts';
import type { ReactNode } from 'react';
import {
  Button,
  Chip,
  EmptyState,
  Icon,
  Inset,
  InsetRow,
  Mark,
  MenuButton,
  MenuItem,
  PropertyGroup,
  PropertyRow,
  SheetDialog,
  StoreMark,
} from '../components';
import type { FilterKind } from '../components';
import {
  canChangeItem,
  displayPath,
  displayPathComponent,
  fmtSize,
  formatRole,
  hue as nameHue,
  isLogin,
  kindLabel,
  kindOf,
  nameOf,
  parseRole,
  partiesOf,
  partyName,
  prefixOf,
  readersOf,
  serverDisplayLabelForStore,
  storeAvailability,
  storeOf,
} from '../model';
import type { Item, Party, RoleWire, AgentSnapshot } from '../model';
import { AccountMark } from './account-switcher';
import type { NavigationGuard, Selection } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemRequest, ReadItemResponse } from '../bridge';
import { useFileDrop } from '../file-drop';
import type { MutationFailureHandler } from '../mutation-recovery';
import { editableValue } from './edit-value';
import { BigTile, itemTile } from './item-tile';
import { accessCopy, itemActionProblem } from './store-access';
import { useConcealOnInactive } from '../use-conceal-on-inactive';

const FIELD_LABELS: Readonly<Record<string, string>> = {
  user: 'User name',
  username: 'User name',
  password: 'Password',
  url: 'Website',
  ssid: 'Network',
};

const MASK = '••••••••••••';
const systemAccessNow = (): number => Date.now() / 1000;

/** Coalesces concurrent in-flight read requests for the same item. */
const readFlights = new WeakMap<
  Bridge,
  WeakMap<object, Map<string, Promise<ReadItemResponse>>>
>();

function readOnce(
  bridge: Bridge,
  request: ItemRequest,
  accessGeneration: number,
  accessSession: object,
): Promise<ReadItemResponse> {
  const key = JSON.stringify([
    accessGeneration,
    request.storeId,
    request.path,
    request.version,
  ]);
  const sessions =
    readFlights.get(bridge) ??
    new WeakMap<object, Map<string, Promise<ReadItemResponse>>>();
  readFlights.set(bridge, sessions);
  const flights =
    sessions.get(accessSession) ?? new Map<string, Promise<ReadItemResponse>>();
  sessions.set(accessSession, flights);
  const current = flights.get(key);
  if (current) return current;
  const pending = bridge.readItem(request).finally(() => flights.delete(key));
  flights.set(key, pending);
  return pending;
}

function assertExactRead(
  request: ItemRequest,
  response: ReadItemResponse,
): void {
  if (
    response.store !== request.storeId ||
    response.path !== request.path ||
    response.version !== request.version
  ) {
    throw new Error(
      'Received content for a different item or version than requested.',
    );
  }
}

/** "Owner" / "Member · visibility 0", from either shape of the wire. */
function roleText(wire: RoleWire): string {
  const role = parseRole(wire);
  if (role) return formatRole(role);
  return typeof wire === 'string' ? wire : wire.role;
}

/** Parses lines formatted as "key: value". Lines without a colon delimiter are treated as raw values. */
function parseFields(value: string): [string | null, string][] {
  return value.split('\n').map((line) => {
    const cut = line.indexOf(': ');
    return cut > 0
      ? ([line.slice(0, cut), line.slice(cut + 2)] as [string, string])
      : ([null, line] as [null, string]);
  });
}

interface PasswordField {
  field: string | null;
  value: string;
  secret: boolean;
}

/**
 * Maps raw or masked password values to displayable field rows.
 */
function passwordFields(
  masked: string | undefined,
  shown: string | null,
): PasswordField[] {
  if (shown !== null) {
    const learned = parseFields(shown);
    if (learned.some(([field]) => field === 'password')) {
      return learned.map(([field, value]) => ({
        field,
        value,
        secret: field === 'password',
      }));
    }
    if (!masked) return [{ field: null, value: shown, secret: true }];
    return parseFields(masked).map(([field, value]) => ({
      field,
      value: field === 'password' ? shown : value,
      secret: field === 'password',
    }));
  }
  if (!masked)
    return [{ field: 'password', value: '••••••••••', secret: true }];
  return parseFields(masked).map(([field, value]) => ({
    field,
    value: field === 'password' ? '••••••••••' : value,
    secret: field === 'password',
  }));
}

/** The sanctioned wording for a server whose check-in has lapsed. */
const LEASE_LAPSED =
  'This server is locked because its connection expired. Items on this server cannot be viewed or edited until the connection is renewed. Other servers remain available.';

function PartyRow({ party }: { party: Party }): ReactNode {
  const name = partyName(party);
  const team = party.party_kind !== 'user';
  return (
    <div className="party">
      {team ? (
        <Mark hue={nameHue(name)} size="md">
          {name.slice(0, 1).toUpperCase()}
        </Mark>
      ) : (
        <AccountMark name={name} />
      )}
      <span className="nm">
        <span className="t">{name}</span>
        {party.label ? <Chip tone="you">you</Chip> : null}
        {team ? <Chip>Team</Chip> : null}
      </span>
      <Chip>{roleText(party.destination_role)}</Chip>
    </div>
  );
}

export interface DetailsPanelProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  selection: Selection;
  /** Key of an item requested to be revealed immediately on render. */
  revealRequest?: string | null;
  onRevealHandled?: () => void;
  onClose: () => void;
  onDelete: (item: Item) => void;
  onMove?: (item: Item) => void;
  /** Opens a folder of the item's store from the location bar (`path` "/" is the root). */
  onOpenFolder?: (storeId: string, path: string) => void;
  /** Opens the server page for a store that cannot be read (Check Status). */
  onOpenServer?: (profile: string) => void;
  onConflict: (
    item: Item,
    draft: string,
    operation?: 'edit' | 'replace',
  ) => void;
  /** Pass the affected profile so post-mutation refresh can reload only that profile. */
  onApplied: (message: string, profile?: string) => Promise<void>;
  onCommandError: (error: unknown, item?: Item) => void;
  onMutationError: MutationFailureHandler;
  /** Signal counter incremented to clear revealed secrets from view state. */
  concealSignal?: number;
  /** Changes whenever access is revoked so old read flights cannot be reused. */
  accessGeneration?: number;
  /** Current wall-clock seconds used by access checks at dispatch/result time. */
  accessNow?: () => number;
  /** Unique unlocked-shell identity preventing reads from crossing lock cycles. */
  accessSession?: object;
  accessTicket?: import('../app/access-lifetime').AccessTicket;
  resumeDraft?: {
    store: string;
    path: string;
    value: string;
    epoch: number;
  } | null;
}

export function DetailsPanel({
  snapshot,
  bridge,
  selection,
  revealRequest = null,
  onRevealHandled,
  onClose,
  onDelete,
  onMove,
  onOpenFolder,
  onOpenServer,
  onConflict,
  onApplied,
  onCommandError,
  onMutationError,
  concealSignal = 0,
  accessGeneration = 0,
  accessNow = systemAccessNow,
  accessSession,
  accessTicket,
  resumeDraft = null,
}: DetailsPanelProps): ReactNode {
  // The panel is a column of the window, not a popover: a click elsewhere
  // does not close it. Escape here, or a click on the list's empty space,
  // clears the selection through the guarded `onClose`.
  const panelRef = useRef<HTMLElement>(null);
  const item: Item | undefined = selection
    ? snapshot.items.find(
        (candidate) =>
          candidate.store === selection.store &&
          candidate.path === selection.path,
      )
    : undefined;

  const key = item ? `${item.store}|${item.path}|${item.version}` : '';
  const [read, setRead] = useState<
    | { key: string; state: 'loading' }
    | { key: string; state: 'shown'; value: string }
    | { key: string; state: 'error'; message: string }
    | null
  >(null);
  const toasts = useToast();
  const accessDescriptionId = useId();
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState('');
  const [editPasswordShown, setEditPasswordShown] = useState(false);
  const [editContentConcealed, setEditContentConcealed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [replacementPath, setReplacementPath] = useState<string | null>(null);
  const [dropHover, setDropHover] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);
  const [binaryFile, setBinaryFile] = useState(false);
  const focusEditorOnOpen = useRef(false);
  const focusActionsOnClose = useRef(false);
  const editorRef = useRef<HTMLDivElement>(null);
  const concealEpoch = useRef(0);
  // The value the editor opened from, against which `editValue` is unsaved
  // work. `null` while no editor is open, and for a draft restored from a
  // conflict, whose stored counterpart is by definition not what it holds.
  const editBaseline = useRef<string | null>(null);
  const editTarget = useRef<Item | null>(null);
  const editScope = useRef('');
  const editGeneration = useRef(accessGeneration);
  const selectedStore = item ? storeOf(snapshot, item.store) : undefined;
  const selectedServer = snapshot.servers.find(
    (server) => server.profileName === selectedStore?.server,
  );
  const selectedAccount = snapshot.accounts.find(
    (account) =>
      account.server === selectedStore?.server &&
      account.alias === selectedStore?.account,
  );
  const scopeIdentity = JSON.stringify([
    item?.store,
    item?.path,
    item?.kind,
    selectedServer?.host_id,
    selectedServer?.configuredEndpoint,
    selectedAccount?.username,
  ]);
  // Epoch of the retained draft already restored, preventing duplicate restores on refresh.
  const appliedDraft = useRef<number | null>(null);
  // Current item key ref to avoid stale closures in asynchronous callbacks.
  const keyRef = useRef(key);
  keyRef.current = key;
  const accessGenerationRef = useRef(accessGeneration);
  accessGenerationRef.current = accessGeneration;
  const localAccessSession = useRef<object>({});
  const activeAccessSession = accessSession ?? localAccessSession.current;

  const accessProblem = useCallback(
    (write = false): string | undefined => {
      if (accessTicket && !accessTicket.isCurrent())
        return 'Access changed. Refresh this item before continuing.';
      return item
        ? itemActionProblem(snapshot, item, write, accessNow())
        : 'This item is no longer available. Refresh the vault before continuing.';
    },
    [accessNow, accessTicket, item, snapshot],
  );
  const accessProblemRef = useRef(accessProblem);
  accessProblemRef.current = accessProblem;
  const accessAvailable = useCallback(() => !accessProblemRef.current(), []);
  const requireAccess = useCallback((write = false): boolean => {
    const message = accessProblemRef.current(write);
    if (!message) return true;
    if (write) setEditError(message);
    else setRead({ key: keyRef.current, state: 'error', message });
    return false;
  }, []);
  const readProblem = accessProblem();
  const writeProblem = accessProblem(true);
  useEffect(() => {
    if (!readProblem) return;
    setRead(null);
    setEditPasswordShown(false);
  }, [readProblem]);

  const request = useMemo<ItemRequest | null>(
    () =>
      item
        ? { storeId: item.store, path: item.path, version: item.version }
        : null,
    [item],
  );

  // Keep Show errors in the panel with its retry control. Route Copy, Download,
  // and edit/replace errors to the shell because those actions have no
  // persistent inline error state until their editor or dialog opens.
  const show = useCallback(async () => {
    if (!item || !request) return;
    const requestKey = key;
    const epoch = concealEpoch.current;
    const generation = accessGeneration;
    if (!requireAccess()) return;
    setRead({ key: requestKey, state: 'loading' });
    try {
      const response = await readOnce(
        bridge,
        request,
        generation,
        activeAccessSession,
      );
      assertExactRead(request, response);
      if (
        epoch !== concealEpoch.current ||
        keyRef.current !== requestKey ||
        accessGenerationRef.current !== generation
      )
        return;
      if (!requireAccess()) return;
      setRead({ key: requestKey, state: 'shown', value: response.value });
      return response.value;
    } catch (error) {
      if (
        epoch !== concealEpoch.current ||
        keyRef.current !== requestKey ||
        accessGenerationRef.current !== generation
      )
        return;
      if (!requireAccess()) return;
      const typed = normalizeCommandError(error);
      if (typed.code === 'agent-lost') {
        onCommandError(error, item);
        return;
      }
      if (typed.code === 'not-text' && item.kind === 'Secret') {
        setBinaryFile(true);
        setRead(null);
        return;
      }
      setRead({
        key: requestKey,
        state: 'error',
        message: typed.message,
      });
    }
  }, [
    requireAccess,
    accessGeneration,
    activeAccessSession,
    bridge,
    item,
    key,
    onCommandError,
    request,
  ]);

  useEffect(() => {
    concealEpoch.current += 1;
    setRead(null);
    setEditPasswordShown(false);
    if (
      editTarget.current &&
      editScope.current === scopeIdentity &&
      editGeneration.current === accessGeneration
    ) {
      setEditError(
        'This item changed on the server. Saving will check the original version.',
      );
    } else {
      setEditing(false);
      setEditValue('');
      setEditContentConcealed(false);
      editBaseline.current = null;
      editTarget.current = null;
      setReplacementPath(null);
      setEditError(null);
      setBinaryFile(false);
    }
    return () => {
      concealEpoch.current += 1;
    };
  }, [key, scopeIdentity, accessGeneration]);

  // Reveal content only when explicitly requested.
  useEffect(() => {
    if (!item) return;
    const requested = revealRequest === `${item.store}|${item.path}`;
    if (!requested) return;
    onRevealHandled?.();
    if (read?.key !== key) void show();
  }, [item, key, onRevealHandled, read?.key, revealRequest, show]);

  // Revealed values and plaintext edit drafts are covered without abandoning
  // the details panel or its unsaved work.
  useConcealOnInactive(
    () => {
      concealEpoch.current += 1;
      setRead(null);
      setEditPasswordShown(false);
      if (
        editing &&
        item &&
        !isLogin(item) &&
        item.kind !== 'File' &&
        !binaryFile
      )
        setEditContentConcealed(true);
    },
    Boolean(read) || editing || saving,
  );

  // A read in flight when access changes generation is dropped when it lands,
  // so the control it disabled is released now instead of reading forever.
  useEffect(() => {
    setRead((current) => (current?.state === 'loading' ? null : current));
  }, [accessGeneration]);

  useEffect(() => {
    if (!concealSignal) return;
    concealEpoch.current += 1;
    setRead(null);
    setEditValue('');
    editBaseline.current = null;
    editTarget.current = null;
    setEditing(false);
    setEditPasswordShown(false);
    setEditContentConcealed(false);
    setReplacementPath(null);
  }, [accessGeneration, concealSignal]);

  useEffect(() => {
    if (!item || !resumeDraft) return;
    if (item.store !== resumeDraft.store || item.path !== resumeDraft.path)
      return;
    // Restore the draft only once per refresh epoch to avoid overwriting newer changes.
    if (appliedDraft.current === resumeDraft.epoch) return;
    appliedDraft.current = resumeDraft.epoch;
    setEditValue(resumeDraft.value);
    setEditContentConcealed(false);
    editBaseline.current = null;
    editTarget.current = item;
    editScope.current = scopeIdentity;
    editGeneration.current = accessGeneration;
    setEditing(true);
  }, [item, resumeDraft, scopeIdentity, accessGeneration]);

  useEffect(() => {
    if (!editing || !focusEditorOnOpen.current) return;
    focusEditorOnOpen.current = false;
    // The first field, or Save in the bar when the contents are concealed.
    (
      editorRef.current?.querySelector<HTMLElement>(
        'input:not([type="hidden"]), textarea',
      ) ?? panelRef.current?.querySelector<HTMLElement>('.eact button')
    )?.focus();
  }, [editing]);

  useEffect(() => {
    if (editing || !focusActionsOnClose.current) return;
    focusActionsOnClose.current = false;
    editorRef.current?.querySelector<HTMLElement>('.hero-acts button')?.focus();
  }, [editing]);

  const selectedFileMode = item?.kind === 'File' || binaryFile;
  useFileDrop({
    bridge,
    active: editing && selectedFileMode,
    onHover: setDropHover,
    onPaths: (paths) => {
      setDropHover(false);
      if (paths.length !== 1) {
        setReplacementPath(null);
        setEditError('Drop exactly one file to replace the current contents.');
        return;
      }
      setReplacementPath(paths[0] ?? null);
      setEditError(null);
    },
    onError: (error) => setEditError(normalizeCommandError(error).message),
  });

  /* ------------------------------------------------------- unsaved edits -- */

  /** Takes the editor down, so a confirmed move lands on a clean panel. */
  const clearEdit = useCallback(() => {
    setEditing(false);
    setEditValue('');
    setEditContentConcealed(false);
    editBaseline.current = null;
    editTarget.current = null;
    setEditPasswordShown(false);
    setReplacementPath(null);
    setEditError(null);
  }, []);

  // An open editor holding the stored value is not unsaved work; a chosen
  // replacement file is, since the file editor carries no text of its own.
  const editDirty =
    editing &&
    (replacementPath !== null ||
      editBaseline.current === null ||
      editValue !== editBaseline.current);
  // Read by the guard when it is asked, so typing re-registers nothing.
  const editGuardState = useRef({
    dirty: false,
    name: '',
    selection: null as Selection,
  });
  editGuardState.current = {
    dirty: editDirty,
    name: item ? nameOf(item.path) : '',
    selection: item ? { store: item.store, path: item.path } : null,
  };
  const editGuard = useCallback<NavigationGuard>(
    (intent) => {
      const { dirty, name, selection } = editGuardState.current;
      if (!dirty) return null;
      // Reselecting the item being edited leaves the editor on screen.
      if (
        intent.kind === 'select' &&
        intent.selection &&
        selection &&
        intent.selection.store === selection.store &&
        intent.selection.path === selection.path
      )
        return null;
      return {
        verdict: 'prompt',
        title: 'Discard changes?',
        body: selectedFileMode
          ? `The replacement for ${name} has not been saved.`
          : `Your edits to ${name} have not been saved.`,
        confirm: 'Discard',
        onConfirm: clearEdit,
      };
    },
    [clearEdit, selectedFileMode],
  );
  useNavigationGuard(editGuard);

  if (!item) {
    return (
      <aside
        ref={panelRef}
        className="details details-empty"
        aria-label="Details"
      >
        <div className="bar" data-tauri-drag-region="" />
        <EmptyState title="No Selection" />
      </aside>
    );
  }

  const store = storeOf(snapshot, item.store);
  // A Document is read as a value or as a file by its node kind: a typed value
  // is a Secret, a file brought in from disk is a File.
  const kind = kindOf(item) as FilterKind;
  const fileMode = item.kind === 'File' || binaryFile;
  const replacing = editing && fileMode;
  const inlineEditing = editing && !fileMode;
  const displayKind: FilterKind = kind;
  const team = store?.kind === 'team';
  const saveProblem =
    writeProblem ??
    (editTarget.current &&
    editScope.current === scopeIdentity &&
    editGeneration.current === accessGeneration
      ? undefined
      : fileMode
        ? 'Access changed. Reopen Replace before continuing.'
        : 'Access changed. Reopen the editor before saving.');
  const blockingReason = readProblem ?? (editing ? saveProblem : writeProblem);
  const readers = readersOf(snapshot, item);
  const parties = store ? partiesOf(snapshot, store.id) : [];
  const activeRead = read?.key === key ? read : null;
  const shownValue =
    !readProblem && activeRead?.state === 'shown' ? activeRead.value : null;
  const readError = activeRead?.state === 'error' ? activeRead.message : null;
  const reading = activeRead?.state === 'loading';
  const serverLabel = store ? serverDisplayLabelForStore(snapshot, store) : '';
  const now = accessNow();
  const availability = store
    ? storeAvailability(snapshot, store, { nowSeconds: now })
    : null;
  // The store itself cannot be read (a lapsed check-in, a blocked server):
  // the panel says so in a notice and every action on the item stays
  // disabled with that reason.
  const lockedCopy =
    store && availability && !availability.available
      ? accessCopy(availability.reason, store, serverLabel)
      : null;
  const lockedReason =
    availability && !availability.available ? availability.reason : null;
  // A member who may read but not change the item: the disabled Edit says
  // why in its tooltip, and the description stays out of the layout.
  const roleOnly = Boolean(
    !readProblem &&
    !editing &&
    writeProblem &&
    availability?.available &&
    !(accessTicket && !accessTicket.isCurrent()) &&
    !canChangeItem(snapshot, item, { nowSeconds: now }),
  );
  const describedBy = (problem: string | undefined): string | undefined =>
    problem ? accessDescriptionId : undefined;

  const copyValue = async (): Promise<void> => {
    if (!request || !requireAccess()) return;
    try {
      await bridge.copyItemValue(request);
      toasts.show(`${kind === 'Password' ? 'Password' : 'Value'} copied`);
    } catch (error) {
      onCommandError(error, item);
    }
  };
  // User names and websites are shown in the clear already; they are copied
  // as the text the row draws.
  const copyField = async (label: string, text: string): Promise<void> => {
    if (!requireAccess()) return;
    try {
      await bridge.copyText(text);
      toasts.show(`${label} copied`);
    } catch (error) {
      onCommandError(error, item);
    }
  };
  const copyPath = async (): Promise<void> => {
    if (!request || !requireAccess()) return;
    try {
      await bridge.copyItemPath(request);
      toasts.show('Path copied');
    } catch (error) {
      onCommandError(error, item);
    }
  };
  const hide = (): void => {
    concealEpoch.current += 1;
    setRead(null);
  };

  const beginEdit = async (): Promise<void> => {
    if (!request || !requireAccess(true)) return;
    setEditError(null);
    if (fileMode) {
      setEditContentConcealed(false);
      editBaseline.current = '';
      editTarget.current = item;
      editScope.current = scopeIdentity;
      editGeneration.current = accessGeneration;
      setEditing(true);
      return;
    }
    // Ensure the response corresponds to the currently selected item before opening the editor.
    const requestKey = key;
    const epoch = concealEpoch.current;
    const generation = accessGeneration;
    const current = (): boolean =>
      epoch === concealEpoch.current &&
      requestKey === keyRef.current &&
      generation === accessGenerationRef.current &&
      accessAvailable();
    setSaving(true);
    try {
      const response = await readOnce(
        bridge,
        request,
        generation,
        activeAccessSession,
      );
      assertExactRead(request, response);
      if (!current()) return;
      const opened = editableValue(item, response.value);
      setEditValue(opened);
      setEditContentConcealed(false);
      editBaseline.current = opened;
      editTarget.current = item;
      editScope.current = scopeIdentity;
      editGeneration.current = accessGeneration;
      setEditPasswordShown(false);
      setEditing(true);
      setRead(null);
    } catch (error) {
      if (!current()) return;
      const typed = normalizeCommandError(error);
      if (typed.code === 'not-text' && item.kind === 'Secret') {
        setBinaryFile(true);
        setRead(null);
        setEditContentConcealed(false);
        editBaseline.current = '';
        editTarget.current = item;
        editScope.current = scopeIdentity;
        editGeneration.current = accessGeneration;
        setEditing(true);
      } else {
        onCommandError(error, item);
      }
    } finally {
      setSaving(false);
    }
  };

  const saveEdit = async (): Promise<void> => {
    if (saving || !requireAccess(true)) return;
    const target = editTarget.current;
    if (
      !request ||
      !target ||
      editScope.current !== scopeIdentity ||
      editGeneration.current !== accessGeneration
    ) {
      setEditError(
        fileMode
          ? 'Access changed. Reopen Replace before continuing.'
          : 'Access changed. Reopen the editor before saving.',
      );
      return;
    }
    setEditError(null);
    const editRequest = {
      storeId: target.store,
      path: target.path,
      version: target.version,
      direntId: target.direntId,
    };
    setSaving(true);
    try {
      if (fileMode) {
        const response = replacementPath
          ? await bridge.replaceDroppedFile({
              ...editRequest,
              sourcePath: replacementPath,
            })
          : await bridge.pickAndReplaceFile(editRequest);
        if (!response.applied) return;
      } else {
        await bridge.editTextItem({ ...editRequest, value: editValue });
      }
      editBaseline.current = null;
      editTarget.current = null;
      if (!fileMode) focusActionsOnClose.current = true;
      setEditing(false);
      await onApplied(
        fileMode ? 'File replaced' : 'Changes saved',
        storeOf(snapshot, target.store)?.server,
      );
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'conflict') {
        if (fileMode) clearEdit();
        onConflict(target, editValue, fileMode ? 'replace' : 'edit');
        await onMutationError(error, { report: false });
      } else {
        await onMutationError(error, { item: target });
      }
    } finally {
      setSaving(false);
    }
  };

  const downloadFile = (): void => {
    if (!request || !requireAccess()) return;
    void bridge.downloadFile(request).then(
      ({ saved }) =>
        toasts.show(
          saved ? `Downloaded ${nameOf(item.path)}` : 'Download cancelled',
        ),
      (error) => onCommandError(error, item),
    );
  };

  const editFields = editing && isLogin(item) ? parseFields(editValue) : [];
  const updateEditField = (index: number, value: string): void => {
    setEditValue(
      editFields
        .map(([field, current], candidate) => {
          const next = candidate === index ? value : current;
          return field ? `${field}: ${next}` : next;
        })
        .join('\n'),
    );
  };

  const showButton = (
    <Button
      aria-label={reading ? 'Reading…' : 'Show'}
      disabled={reading || Boolean(readProblem)}
      title={readProblem ?? (reading ? 'Reading…' : 'Show')}
      aria-describedby={describedBy(readProblem)}
      onClick={() => void show()}
    >
      Show
    </Button>
  );
  const hideButton = (
    <Button title="Hide" onClick={hide}>
      Hide
    </Button>
  );
  const copyButton = (label: string, run: () => Promise<void>): ReactNode => (
    <Button
      aria-label={`Copy ${label}`}
      disabled={Boolean(readProblem)}
      title={readProblem ?? `Copy ${label}`}
      aria-describedby={describedBy(readProblem)}
      onClick={() => void run()}
    >
      Copy
    </Button>
  );
  const secretValue = (value: string): ReactNode =>
    shownValue === null ? (
      <span className="masked">{MASK}</span>
    ) : (
      <span className="value-text">{value}</span>
    );

  const content: ReactNode =
    inlineEditing && isLogin(item) ? (
      editFields.map(([field, value], index) => {
        const label = FIELD_LABELS[field ?? ''] ?? field ?? 'Value';
        const secret = field?.toLowerCase() === 'password';
        return (
          <PropertyRow
            key={`${field ?? 'value'}-${index}`}
            label={label}
            className="edit"
            actions={
              secret ? (
                <Button
                  aria-label={editPasswordShown ? 'Hide' : 'Show'}
                  disabled={!editPasswordShown && Boolean(readProblem)}
                  title={
                    (!editPasswordShown ? readProblem : undefined) ??
                    (editPasswordShown ? 'Hide' : 'Show')
                  }
                  aria-describedby={
                    !editPasswordShown ? describedBy(readProblem) : undefined
                  }
                  onClick={() => {
                    if (editPasswordShown || requireAccess())
                      setEditPasswordShown((shown) => !shown);
                  }}
                >
                  {editPasswordShown ? 'Hide' : 'Show'}
                </Button>
              ) : undefined
            }
          >
            <input
              className="inp"
              aria-label={label}
              type={secret && !editPasswordShown ? 'password' : 'text'}
              placeholder={secret ? 'password' : undefined}
              value={value}
              onChange={(event) => updateEditField(index, event.target.value)}
            />
          </PropertyRow>
        );
      })
    ) : inlineEditing && editContentConcealed ? (
      <PropertyRow
        label="Value"
        actions={
          <Button title="Show" onClick={() => setEditContentConcealed(false)}>
            Show
          </Button>
        }
      >
        <span className="masked">{MASK}</span>
      </PropertyRow>
    ) : inlineEditing ? (
      <PropertyRow label="Value" className="edit top">
        <textarea
          className="inp mono"
          aria-label="Value"
          rows={4}
          value={editValue}
          onChange={(event) => setEditValue(event.target.value)}
        />
      </PropertyRow>
    ) : kind === 'Password' && !fileMode ? (
      passwordFields(item.value, shownValue).map(
        ({ field, value, secret }, index) => {
          const label = FIELD_LABELS[field ?? ''] ?? field ?? 'Value';
          return secret ? (
            <PropertyRow
              key={index}
              label={label}
              actions={
                <>
                  {shownValue === null ? showButton : hideButton}
                  {copyButton(label, copyValue)}
                </>
              }
            >
              {secretValue(value)}
            </PropertyRow>
          ) : (
            <PropertyRow
              key={index}
              label={label}
              actions={
                value
                  ? copyButton(label, () => copyField(label, value))
                  : undefined
              }
            >
              <span className="clip" title={value}>
                {value}
              </span>
            </PropertyRow>
          );
        },
      )
    ) : kind === 'Document' && !fileMode ? (
      <PropertyRow
        label="Value"
        actions={
          <>
            {shownValue === null ? showButton : hideButton}
            {copyButton('Value', copyValue)}
          </>
        }
      >
        {secretValue(shownValue ?? '')}
      </PropertyRow>
    ) : (
      <PropertyRow label="File">
        <span className="fname">{nameOf(item.path)}</span>
        <Chip>{item.size === null ? 'Encrypted' : fmtSize(item.size)}</Chip>
        {item.size === null ? null : <Chip>Encrypted at rest</Chip>}
      </PropertyRow>
    );

  const name = nameOf(item.path);
  const tile = itemTile(item);
  const folderTrail = prefixOf(item.path).split('/').filter(Boolean);
  const pathStep = (label: ReactNode, path: string): ReactNode =>
    store && onOpenFolder ? (
      <button type="button" onClick={() => onOpenFolder(store.id, path)}>
        {label}
      </button>
    ) : (
      <span className="step">{label}</span>
    );
  const description = blockingReason && !replacing && !lockedCopy;

  return (
    <>
      <aside
        ref={panelRef}
        className="details"
        aria-label={`Details for ${name}`}
        onKeyDown={(event) => {
          // Escape from anywhere in the panel clears the selection, as an
          // outside click does, unless a field or the editor holds it.
          if (event.key !== 'Escape' || event.defaultPrevented || editing)
            return;
          if (
            event.target instanceof Element &&
            event.target.closest('input, textarea, select')
          )
            return;
          event.preventDefault();
          onClose();
        }}
      >
        <div className="bar" data-tauri-drag-region="">
          <nav className="pathbar" aria-label="Location">
            {pathStep(
              <>
                {store ? <StoreMark snapshot={snapshot} store={store} /> : null}
                <span className="t">{store?.name ?? item.store}</span>
              </>,
              '/',
            )}
            {folderTrail.map((part, index) => (
              <Fragment key={index}>
                <Icon name="chevronRight" className="sep" />
                {pathStep(
                  <span className="t">{displayPathComponent(part)}</span>,
                  `/${folderTrail.slice(0, index + 1).join('/')}`,
                )}
              </Fragment>
            ))}
          </nav>
          <span className="sp" />
          {inlineEditing ? (
            <span className="eact">
              <Button
                disabled={saving}
                onClick={() => {
                  focusActionsOnClose.current = true;
                  clearEdit();
                }}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={saving || Boolean(saveProblem)}
                title={saveProblem}
                aria-describedby={describedBy(saveProblem)}
                onClick={() => void saveEdit()}
              >
                {saving ? 'Saving…' : 'Save Changes'}
              </Button>
            </span>
          ) : (
            <MenuButton
              label=""
              aria-label="More actions"
              title="More actions"
              menuLabel={`${name} actions`}
              variant="quiet"
              icon="ellipsis"
              trailingIcon={null}
              align="end"
            >
              {() => (
                <>
                  {fileMode ? (
                    <MenuItem
                      icon="download"
                      reason={readProblem}
                      onClick={downloadFile}
                    >
                      Download
                    </MenuItem>
                  ) : (
                    <>
                      <MenuItem
                        icon={shownValue === null ? 'eye' : 'eyeOff'}
                        reason={shownValue === null ? readProblem : undefined}
                        onClick={() => {
                          if (shownValue === null) void show();
                          else hide();
                        }}
                      >
                        {shownValue === null ? 'Show Value' : 'Hide Value'}
                      </MenuItem>
                      <MenuItem
                        icon="copy"
                        reason={readProblem}
                        onClick={() => void copyValue()}
                      >
                        {kind === 'Password' ? 'Copy Password' : 'Copy Value'}
                      </MenuItem>
                    </>
                  )}
                  <MenuItem
                    icon="folder"
                    reason={readProblem}
                    onClick={() => void copyPath()}
                  >
                    Copy Path
                  </MenuItem>
                  <div className="menu-separator" role="separator" />
                  {onMove ? (
                    <MenuItem
                      icon="pencil"
                      reason={writeProblem}
                      onClick={() => {
                        if (requireAccess(true)) onMove(item);
                      }}
                    >
                      Rename or Move…
                    </MenuItem>
                  ) : null}
                  <MenuItem
                    icon="trash"
                    danger
                    shortcut="⌘⌫"
                    reason={writeProblem}
                    onClick={() => {
                      if (requireAccess(true)) onDelete(item);
                    }}
                  >
                    Delete…
                  </MenuItem>
                </>
              )}
            </MenuButton>
          )}
        </div>
        <div className="scroll" ref={editorRef}>
          <div className="hero">
            <BigTile face={tile} />
            <div className="t">
              <h2 title={name}>{name}</h2>
              <p>
                {kindLabel(displayKind)}
                {serverLabel ? ` · ${serverLabel}` : ''}
              </p>
            </div>
            {inlineEditing ? null : (
              <div className="hero-acts">
                {fileMode ? (
                  <Button
                    icon="download"
                    disabled={Boolean(readProblem)}
                    title={readProblem ?? 'Save a copy to disk'}
                    aria-describedby={describedBy(readProblem)}
                    onClick={downloadFile}
                  >
                    Download
                  </Button>
                ) : null}
                <Button
                  disabled={Boolean(writeProblem) || saving}
                  title={
                    writeProblem ??
                    (fileMode ? 'Replace this file' : 'Edit this item')
                  }
                  aria-describedby={describedBy(writeProblem)}
                  onClick={() => {
                    focusEditorOnOpen.current = true;
                    void beginEdit();
                  }}
                >
                  {saving && !fileMode
                    ? 'Reading…'
                    : fileMode
                      ? 'Replace…'
                      : 'Edit'}
                </Button>
              </div>
            )}
          </div>
          {lockedCopy && blockingReason && !replacing ? (
            <div id={accessDescriptionId} className="notice stop" role="status">
              <Icon name="lock" />
              <span className="t">
                <b>
                  {lockedReason === 'check-in-expired'
                    ? `${serverLabel} is locked.`
                    : `${lockedCopy.title}.`}
                </b>
                {lockedReason === 'check-in-expired'
                  ? LEASE_LAPSED
                  : blockingReason}
              </span>
              {store && onOpenServer && lockedCopy.action === 'open-server' ? (
                <Button onClick={() => onOpenServer(store.server)}>
                  Check Status
                </Button>
              ) : null}
            </div>
          ) : description ? (
            <p
              id={accessDescriptionId}
              className={roleOnly ? 'offscreen' : 'action-error'}
              role="status"
            >
              {blockingReason}
            </p>
          ) : null}
          {inlineEditing ? (
            <div className="notice info">
              <Icon name="info" />
              <span className="t">
                Saves version {item.version + 1} if no one else has changed this
                item since version {item.version}.
              </span>
            </div>
          ) : null}
          <PropertyGroup className="content">{content}</PropertyGroup>
          {readError && readError !== blockingReason ? (
            <p className="action-error" role="alert">
              {readError}
            </p>
          ) : null}
          {!replacing && editError && editError !== blockingReason ? (
            <p className="action-error" role="alert">
              {editError}
            </p>
          ) : null}
          <PropertyGroup>
            <PropertyRow label="Vault">
              {store ? <StoreMark snapshot={snapshot} store={store} /> : null}
              <span>{store?.name ?? item.store}</span>
              {serverLabel ? (
                <span className="muted">· {serverLabel}</span>
              ) : null}
            </PropertyRow>
            <PropertyRow label="Path" mono>
              {displayPath(item.path)}
            </PropertyRow>
            <PropertyRow label="Version">{item.version}</PropertyRow>
            <PropertyRow label="Size">{fmtSize(item.size)}</PropertyRow>
          </PropertyGroup>
          <PropertyGroup>
            <PropertyRow label="Read access">{roleText(item.read)}</PropertyRow>
            <PropertyRow label="Write access">
              {roleText(item.write)}
            </PropertyRow>
          </PropertyGroup>
          <PropertyGroup label="Sharing" className="sharing">
            {!team ? (
              <p className="note">
                Your {store?.name ?? 'personal'} vault is private and cannot be
                shared. To share an item, move it to a team vault.
              </p>
            ) : readers ? (
              <>
                <p className="note">
                  {readers.length} of {parties.length} in {store?.name} with{' '}
                  <b>{roleText(item.read)}</b> access or higher can read this
                  item.
                </p>
                <div className="roster">
                  {readers.map((party) => (
                    <PartyRow key={party.party_id_hex} party={party} />
                  ))}
                </div>
              </>
            ) : (
              <p className="note">
                Members of {store?.name} with <b>{roleText(item.read)}</b>{' '}
                access or higher can read this item.
              </p>
            )}
          </PropertyGroup>

          {/* Raw metadata is available only in developer diagnostics. */}
        </div>
      </aside>
      {replacing ? (
        <SheetDialog
          title={`Replace ${name}`}
          glyph={<BigTile face={tile} />}
          onClose={clearEdit}
          dismissible={!saving}
          footer={
            <>
              <Button disabled={saving} onClick={clearEdit}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={saving || Boolean(saveProblem)}
                title={saveProblem}
                aria-describedby={describedBy(saveProblem)}
                onClick={() => void saveEdit()}
              >
                {saving
                  ? 'Replacing…'
                  : replacementPath
                    ? 'Replace File'
                    : 'Choose File…'}
              </Button>
            </>
          }
        >
          <p>
            Choose a file to replace the contents of <b>{name}</b>. The
            replacement is saved only if no one else has changed the item since
            version {item.version}.
          </p>
          <Inset className={dropHover ? 'drop-hover' : undefined}>
            <InsetRow label="File">
              <span className={replacementPath ? 'mono' : 'dim'}>
                {replacementPath
                  ? replacementPath.split(/[\\/]/).at(-1)
                  : dropHover
                    ? 'Release to use this file'
                    : 'Drop one file here, or choose one below'}
              </span>
            </InsetRow>
          </Inset>
          {editError && editError !== saveProblem ? (
            <p className="action-error" role="alert">
              {editError}
            </p>
          ) : null}
          {saveProblem ? (
            <p id={accessDescriptionId} className="action-error" role="status">
              {saveProblem}
            </p>
          ) : null}
        </SheetDialog>
      ) : null}
    </>
  );
}
