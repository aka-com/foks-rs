/**
 * Details panel displaying item metadata, content preview, sharing roster, and item actions.
 */

import {
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
  Field,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  SectionLabel,
} from '../components';
import type { FilterKind } from '../components';
import {
  fmtSize,
  formatRole,
  initials,
  isLogin,
  kindLabel,
  kindOf,
  nameOf,
  parseRole,
  partiesOf,
  partyName,
  peopleLabel,
  prefixOf,
  readersOf,
  storeHues,
  storeNavigationOrder,
  storeOf,
  usernameOf,
} from '../model';
import type { Item, Party, RoleWire, Store, AgentSnapshot } from '../model';
import { AccountMark } from './account-switcher';
import type { NavigationGuard, Selection } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemRequest, ReadItemResponse } from '../bridge';
import { useFileDrop } from '../file-drop';
import type { MutationFailureHandler } from '../mutation-recovery';
import { editableValue } from './edit-value';
import { itemActionProblem } from './store-access';
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

/** Renders the store icon or avatar, matching the style used in the browser tree and list rows. */
function StoreMark({
  snapshot,
  store,
}: {
  snapshot: AgentSnapshot;
  store: Store;
}): ReactNode {
  return store.kind === 'team' ? (
    <span
      className="av team"
      style={{
        background: storeHues(storeNavigationOrder(snapshot)).get(store.id),
      }}
      aria-hidden="true"
    >
      {initials(store.name)}
    </span>
  ) : (
    <AccountMark name={usernameOf(snapshot, store) ?? store.account} />
  );
}

function PartyRow({
  party,
  canRead,
}: {
  party: Party;
  canRead: boolean;
}): ReactNode {
  const details = [
    party.party_kind !== 'user' ? 'Member team' : '',
    canRead ? '' : 'No read access',
  ].filter(Boolean);
  return (
    <div className="party">
      <span className="t">
        {partyName(party)}
        {party.label ? (
          <>
            {' '}
            <Chip tone="you">you</Chip>
          </>
        ) : null}
        {details.length ? <small>{details.join(' · ')}</small> : null}
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
  onConflict: (item: Item, draft: string) => void;
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
  const panelRef = useRef<HTMLElement>(null);
  useEffect(() => {
    const dismiss = (event: PointerEvent): void => {
      const target = event.target;
      if (!(target instanceof Element) || event.button !== 0) return;
      if (
        panelRef.current?.contains(target) ||
        document.querySelector('[role="dialog"], [role="alertdialog"]') ||
        target.closest('[role="menu"], [role="listbox"]')
      )
        return;
      onClose();
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [onClose]);
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
    (server) => server.id === selectedStore?.server,
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
    selectedServer?.configuredProbe,
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
  // and Edit errors to the shell because those actions have no persistent inline
  // error state.
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
        body: `Your edits to ${name} have not been saved.`,
        confirm: 'Discard',
        onConfirm: clearEdit,
      };
    },
    [clearEdit],
  );
  useNavigationGuard(editGuard);

  if (!item) {
    return (
      <aside ref={panelRef} className="details" aria-label="Details">
        <div className="dh">
          <span className="t">
            <h2>Details</h2>
          </span>
          <button
            type="button"
            className="x"
            aria-label="Close"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        </div>
        <div className="scroll">
          <div className="empty" style={{ paddingTop: '60px' }}>
            <span className="big">
              <Icon name="search" />
            </span>
            <h2>No selection</h2>
            <p>Select an item to view its details.</p>
          </div>
        </div>
      </aside>
    );
  }

  const store = storeOf(snapshot, item.store);
  // A Document is read as a value or as a file by its node kind: a typed value
  // is a Secret, a file brought in from disk is a File.
  const kind = kindOf(item) as FilterKind;
  const fileMode = item.kind === 'File' || binaryFile;
  const displayKind: FilterKind = kind;
  const team = store?.kind === 'team';
  const saveProblem =
    writeProblem ??
    (editTarget.current &&
    editScope.current === scopeIdentity &&
    editGeneration.current === accessGeneration
      ? undefined
      : 'Access changed. Reopen the editor before saving.');
  const blockingReason = readProblem ?? (editing ? saveProblem : writeProblem);
  const readers = readersOf(snapshot, item);
  const parties = store ? partiesOf(snapshot, store.id) : [];
  const activeRead = read?.key === key ? read : null;
  const shownValue =
    !readProblem && activeRead?.state === 'shown' ? activeRead.value : null;
  const readError = activeRead?.state === 'error' ? activeRead.message : null;
  const reading = activeRead?.state === 'loading';
  const copyValue = async (): Promise<void> => {
    if (!request || !requireAccess()) return;
    try {
      await bridge.copyItemValue(request);
      toasts.show(`${kind === 'Password' ? 'Password' : 'Value'} copied`);
    } catch (error) {
      onCommandError(error, item);
    }
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
      setEditError('Access changed. Reopen the editor before saving.');
      return;
    }
    setEditError(null);
    const editRequest = {
      storeId: target.store,
      path: target.path,
      version: target.version,
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
      setEditing(false);
      await onApplied('Changes saved', storeOf(snapshot, target.store)?.server);
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'conflict') {
        onConflict(target, editValue);
        await onMutationError(error, { report: false });
      } else {
        await onMutationError(error, { item: target });
      }
    } finally {
      setSaving(false);
    }
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

  const preview =
    editing && fileMode ? (
      <Inset variant="preview" className={dropHover ? 'drop-hover' : undefined}>
        <InsetRow label="Replace">
          <span className={replacementPath ? 'mono' : 'dim'}>
            {replacementPath
              ? replacementPath.split(/[\\/]/).at(-1)
              : dropHover
                ? 'Release to replace file'
                : 'Drop a file here, or choose a file when saving'}
          </span>
        </InsetRow>
      </Inset>
    ) : editing && isLogin(item) ? (
      <Inset className="edit">
        {editFields.map(([field, value], index) => {
          const secret = field?.toLowerCase() === 'password';
          return (
            <Field
              key={`${field ?? 'value'}-${index}`}
              label={FIELD_LABELS[field ?? ''] ?? field ?? 'Value'}
              value={value}
              type={secret && !editPasswordShown ? 'password' : 'text'}
              placeholder={secret ? 'password' : undefined}
              onChange={(next) => updateEditField(index, next)}
              action={
                secret ? (
                  <button
                    className="value-action"
                    aria-label={editPasswordShown ? 'Hide' : 'Show'}
                    type="button"
                    disabled={!editPasswordShown && Boolean(readProblem)}
                    title={
                      (!editPasswordShown ? readProblem : undefined) ??
                      (editPasswordShown ? 'Hide' : 'Show')
                    }
                    aria-describedby={
                      !editPasswordShown && readProblem
                        ? accessDescriptionId
                        : undefined
                    }
                    onClick={() => {
                      if (editPasswordShown || requireAccess())
                        setEditPasswordShown((shown) => !shown);
                    }}
                  >
                    <Icon name={editPasswordShown ? 'eyeOff' : 'eye'} />
                  </button>
                ) : undefined
              }
            />
          );
        })}
      </Inset>
    ) : editing && editContentConcealed ? (
      <Inset variant="preview">
        <InsetRow
          label="Contents"
          valueClass="mask"
          action={
            <button
              title="Show"
              className="value-action"
              aria-label="Show"
              type="button"
              onClick={() => setEditContentConcealed(false)}
            >
              <Icon name="eye" />
            </button>
          }
        >
          {MASK}
        </InsetRow>
      </Inset>
    ) : editing ? (
      <Inset variant="preview">
        <textarea
          aria-label="Contents"
          value={editValue}
          onChange={(event) => setEditValue(event.target.value)}
        />
      </Inset>
    ) : kind === 'Password' && !fileMode ? (
      <Inset variant="preview">
        {passwordFields(item.value, shownValue).map(
          ({ field, value, secret }, index) =>
            secret ? (
              <InsetRow
                key={index}
                label={FIELD_LABELS[field ?? ''] ?? field ?? 'Value'}
                className="rev"
                valueClass={shownValue === null ? 'mask' : 'value-text'}
                action={
                  shownValue === null ? (
                    <>
                      <button
                        className="value-action"
                        aria-label={reading ? 'Reading…' : 'Show'}
                        type="button"
                        disabled={reading || Boolean(readProblem)}
                        title={readProblem ?? (reading ? 'Reading…' : 'Show')}
                        aria-describedby={
                          readProblem ? accessDescriptionId : undefined
                        }
                        onClick={() => void show()}
                      >
                        <Icon name="eye" />
                      </button>
                      <button
                        className="value-action"
                        aria-label="Copy"
                        type="button"
                        disabled={Boolean(readProblem)}
                        title={readProblem ?? 'Copy'}
                        aria-describedby={
                          readProblem ? accessDescriptionId : undefined
                        }
                        onClick={() => void copyValue()}
                      >
                        <Icon name="copy" />
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        title="Hide"
                        className="value-action"
                        aria-label="Hide"
                        type="button"
                        onClick={() => {
                          concealEpoch.current += 1;
                          setRead(null);
                        }}
                      >
                        <Icon name="eyeOff" />
                      </button>
                      <button
                        className="value-action"
                        aria-label="Copy"
                        type="button"
                        disabled={Boolean(readProblem)}
                        title={readProblem ?? 'Copy'}
                        aria-describedby={
                          readProblem ? accessDescriptionId : undefined
                        }
                        onClick={() => void copyValue()}
                      >
                        <Icon name="copy" />
                      </button>
                    </>
                  )
                }
              >
                {shownValue === null ? MASK : value}
              </InsetRow>
            ) : (
              <InsetRow key={index} label={FIELD_LABELS[field ?? ''] ?? field}>
                {value}
              </InsetRow>
            ),
        )}
      </Inset>
    ) : kind === 'Document' && !fileMode ? (
      <Inset variant="preview">
        <InsetRow
          className="rev"
          label="Value"
          valueClass={shownValue === null ? 'mask' : 'value-text'}
          action={
            shownValue === null ? (
              <>
                <button
                  className="value-action"
                  aria-label={reading ? 'Reading…' : 'Show'}
                  type="button"
                  disabled={reading || Boolean(readProblem)}
                  title={readProblem ?? (reading ? 'Reading…' : 'Show')}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={() => void show()}
                >
                  <Icon name="eye" />
                </button>
                <button
                  className="value-action"
                  aria-label="Copy"
                  type="button"
                  disabled={Boolean(readProblem)}
                  title={readProblem ?? 'Copy'}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={() => void copyValue()}
                >
                  <Icon name="copy" />
                </button>
              </>
            ) : (
              <>
                <button
                  title="Hide"
                  className="value-action"
                  aria-label="Hide"
                  type="button"
                  onClick={() => {
                    concealEpoch.current += 1;
                    setRead(null);
                  }}
                >
                  <Icon name="eyeOff" />
                </button>
                <button
                  className="value-action"
                  aria-label="Copy"
                  type="button"
                  disabled={Boolean(readProblem)}
                  title={readProblem ?? 'Copy'}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={() => void copyValue()}
                >
                  <Icon name="copy" />
                </button>
              </>
            )
          }
        >
          {shownValue ?? MASK}
        </InsetRow>
      </Inset>
    ) : (
      // The file card states what the item is; Download sits in the actions
      // row above with every other action on this item.
      <Inset variant="preview">
        <div className="filedoc">
          <span className="big">
            <Icon name="file" />
          </span>
          <span className="t">
            <b>{nameOf(item.path)}</b>
            <small>{fmtSize(item.size)} · encrypted at rest</small>
          </span>
        </div>
      </Inset>
    );

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

  const folderTrail = prefixOf(item.path).split('/').filter(Boolean);
  const where = [store?.name ?? item.store, ...folderTrail].join(' › ');

  return (
    <aside
      ref={panelRef}
      className="details"
      aria-label={`Details for ${nameOf(item.path)}`}
    >
      <div className="dh">
        <KindIcon kind={displayKind} />
        <span className="t">
          <h2>{nameOf(item.path)}</h2>
          <small title={item.path}>{where}</small>
        </span>
        <button
          type="button"
          className="x"
          title="Close"
          aria-label="Close"
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <div className="scroll">
        {blockingReason ? (
          <p id={accessDescriptionId} className="action-error" role="status">
            {blockingReason}
          </p>
        ) : null}
        {/* Item actions; value controls live beside the value. */}
        {editing ? null : (
          <div className="detail-actions">
            {fileMode ? (
              <Button
                icon="download"
                disabled={Boolean(readProblem)}
                title={readProblem ?? 'Save a copy to disk'}
                aria-describedby={readProblem ? accessDescriptionId : undefined}
                onClick={downloadFile}
              >
                Download
              </Button>
            ) : null}
            <Button
              icon="pencil"
              disabled={Boolean(writeProblem) || saving}
              title={writeProblem ?? 'Edit this item'}
              aria-describedby={writeProblem ? accessDescriptionId : undefined}
              onClick={() => void beginEdit()}
            >
              {saving ? 'Reading…' : 'Edit'}
            </Button>
            <Button
              variant="danger"
              icon="trash"
              disabled={Boolean(writeProblem)}
              title={writeProblem ?? 'Delete this item'}
              aria-describedby={writeProblem ? accessDescriptionId : undefined}
              onClick={() => {
                if (requireAccess(true)) onDelete(item);
              }}
            >
              Delete
            </Button>
          </div>
        )}
        {editing || displayKind === 'Password' ? (
          <SectionLabel>{editing ? 'Edit' : 'Fields'}</SectionLabel>
        ) : null}
        {preview}
        {editing ? (
          <div className="pfn">
            {`This item will save as version ${item.version + 1}. If it was modified elsewhere, refresh to review updates before saving.`}
          </div>
        ) : null}
        {readError && readError !== blockingReason ? (
          <p className="action-error" role="alert">
            {readError}
          </p>
        ) : null}
        {editError && editError !== blockingReason ? (
          <p className="action-error" role="alert">
            {editError}
          </p>
        ) : null}
        {editing ? (
          <div className="eact">
            <Button disabled={saving} onClick={clearEdit}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={saving || Boolean(saveProblem)}
              title={saveProblem}
              aria-describedby={saveProblem ? accessDescriptionId : undefined}
              onClick={() => void saveEdit()}
            >
              {saving ? 'Saving…' : 'Save changes'}
            </Button>
          </div>
        ) : null}

        <SectionLabel>Details</SectionLabel>
        <Inset>
          <InsetRow label="Kind">{kindLabel(displayKind)}</InsetRow>
          <InsetRow label="Location" valueClass="mark">
            {store ? <StoreMark snapshot={snapshot} store={store} /> : null}
            <span title={item.path}>{where}</span>
          </InsetRow>
          <InsetRow label="Path">{item.path}</InsetRow>
          <InsetRow label="Version">{item.version}</InsetRow>
          <InsetRow label="Size">{fmtSize(item.size)}</InsetRow>
        </Inset>

        <SectionLabel>Access</SectionLabel>
        <div className="pillrow">
          <span className="pill">
            <Icon name="eye" />
            Read: {roleText(item.read)}
          </span>
          <span className="pill write-access">
            <Icon name="pencil" />
            Write: {roleText(item.write)}
          </span>
          {team ? (
            <span className="pill">
              <Icon name="users" />
              Shared
            </span>
          ) : null}
        </div>

        <div className="who">
          <SectionLabel>Sharing</SectionLabel>
          {team && readers ? (
            <>
              <p>
                {peopleLabel(readers.length)} in {store?.name} with{' '}
                <b>{roleText(item.read)}</b> access or higher can read this
                item.
              </p>
              {parties.map((party) => (
                <PartyRow
                  key={party.party_id_hex}
                  party={party}
                  canRead={readers.includes(party)}
                />
              ))}
            </>
          ) : (
            <p>No one else has access to this item.</p>
          )}
        </div>

        {/* Raw metadata is available only in developer diagnostics. */}
      </div>
    </aside>
  );
}
