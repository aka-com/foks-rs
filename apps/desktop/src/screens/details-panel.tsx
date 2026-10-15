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
import { useToast } from '../../kit/toasts';
import type { ReactNode } from 'react';
import {
  Button,
  Field,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  PasswordInput,
  PartyRow,
  SectionLabel,
  SheetDialog,
  StoreMark,
} from '../components';
import type { FilterKind } from '../components';
import {
  displayPath,
  fieldLineText,
  fmtSize,
  isBlankLine,
  kindLabel,
  kindOf,
  LOGIN_FIELDS,
  loginField,
  loginFieldOf,
  loginLines,
  nameOf,
  partiesOf,
  peopleLabel,
  prefixOf,
  readersOf,
  roleText,
  storeHues,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type { Item, LoginFieldName, AgentSnapshot } from '../model';
import type { NavigationGuard, Selection } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { normalizeCommandError } from '../bridge';
import type {
  Bridge,
  ItemField,
  ItemRequest,
  ReadItemResponse,
} from '../bridge';
import { useFileDrop } from '../file-drop';
import type { MutationFailureHandler } from '../mutation-recovery';
import { COPY_LABELS, copyAndAnnounce } from './item-copy';
import {
  isFavorite,
  itemRefOf,
  toggleFavorite,
  useRecentFavorites,
} from '../recent-favorites';
import { itemActionProblem } from './store-access';
import { copyProblem } from './item-moves';
import { useConcealOnInactive } from '../use-conceal-on-inactive';
import { editorBound, useItemSecrets } from './item-secrets';

const FIELD_LABELS: ReadonlyMap<string, string> = new Map([
  ['user', 'User name'],
  ['username', 'User name'],
  ['password', 'Password'],
  ['url', 'Website'],
  ['website', 'Website'],
  ['ssid', 'Network'],
]);

/**
 * The label of a login line's key: a known field's name, or the key itself.
 * A map lookup, so a key such as `constructor` is not read from a prototype.
 */
function fieldLabel(key: string): string {
  return FIELD_LABELS.get(key) ?? key;
}

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

/**
 * The lines the structured login editor lists, by index, or null when the
 * text opens in the textarea. The editor opens for text with a line the host
 * would copy as the password, which the editor masks, and lists every line
 * but blank ones: typing into a blank line would add a line that is not a
 * field line, and the agent would then list the item as a Document. Blank
 * lines are still saved as they were read. Both choices are made once, when
 * the edit opens, so typing never swaps editors or removes a row.
 */
function loginEditLines(text: string): readonly number[] | null {
  const lines = loginLines(text);
  if (!lines.some(({ key }) => loginFieldOf(key) === 'password')) return null;
  return lines.flatMap((line, index) => (isBlankLine(line) ? [] : [index]));
}

/** One row of a login's Fields section. */
interface LoginRow {
  /** Identifies the row's Show state: a login field, or `line:<index>`. */
  id: string;
  label: string;
  /** The field the host copies for this row; null for other lines. */
  field: LoginFieldName | null;
  /** Null before a read, and for a login field the text lacks. */
  value: string | null;
}

/**
 * The rows a login lists: user name, password and website always, each
 * masked until shown, then after a read any other line, such as a Wi-Fi
 * network name, in stored order. Values come from `loginField`, the parser
 * the host's field copy follows, so a row and its Copy agree.
 */
function loginRows(shown: string | null): LoginRow[] {
  const rows: LoginRow[] = LOGIN_FIELDS.map((field) => ({
    id: field,
    label: fieldLabel(field),
    field,
    value: shown === null ? null : loginField(shown, field),
  }));
  if (shown === null) return rows;
  loginLines(shown).forEach(({ key, value }, index) => {
    if (loginFieldOf(key) !== null || !value) return;
    rows.push({
      id: `line:${index}`,
      label: key === null ? 'Text' : fieldLabel(key),
      field: null,
      value,
    });
  });
  return rows;
}

export interface DetailsPanelProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  selection: Selection;
  /** Key of an item requested to be revealed immediately on render. */
  revealRequest?: string | null;
  onRevealHandled?: () => void;
  onClose: () => void;
  /** Moves the item to Trash, from where it can be restored. */
  onTrash: (item: Item) => void;
  onMove?: (item: Item) => void;
  /** Copies the item into another vault. */
  onCopyToVault?: (item: Item) => void;
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
  onTrash,
  onMove,
  onCopyToVault,
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
  // Every revealed value and edit draft, and each transition that clears
  // them; see item-secrets.ts.
  const {
    secrets: { read, revealed, editor, editError, binaryFile },
    dispatch,
    epoch: concealEpoch,
  } = useItemSecrets();
  const editing = editor !== null;
  const editValue = editor?.value ?? '';
  const editPasswordShown = editor?.passwordShown ?? false;
  const replacementPath = editor?.replacementPath ?? null;
  const toasts = useToast();
  const recentFavorites = useRecentFavorites();
  const accessDescriptionId = useId();
  const [saving, setSaving] = useState(false);
  const [dropHover, setDropHover] = useState(false);
  const focusEditorOnOpen = useRef(false);
  const focusActionsOnClose = useRef(false);
  const editorRef = useRef<HTMLDivElement>(null);
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
  const requireAccess = useCallback(
    (write = false): boolean => {
      const message = accessProblemRef.current(write);
      if (!message) return true;
      if (write) dispatch({ type: 'edit-error', message });
      else dispatch({ type: 'reveal-fail', key: keyRef.current, message });
      return false;
    },
    [dispatch],
  );
  const readProblem = accessProblem();
  const writeProblem = accessProblem(true);
  useEffect(() => {
    if (readProblem) dispatch({ type: 'read-blocked' });
  }, [dispatch, readProblem]);

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
    dispatch({ type: 'reveal-start', key: requestKey });
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
      dispatch({ type: 'reveal-done', key: requestKey, value: response.value });
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
        dispatch({ type: 'not-text' });
        return;
      }
      dispatch({
        type: 'reveal-fail',
        key: requestKey,
        message: typed.message,
      });
    }
  }, [
    requireAccess,
    accessGeneration,
    activeAccessSession,
    bridge,
    concealEpoch,
    dispatch,
    item,
    key,
    onCommandError,
    request,
  ]);

  // A draft still bound to this scope and generation outlives a new version.
  useEffect(() => {
    dispatch({
      type: 'item-changed',
      scope: scopeIdentity,
      generation: accessGeneration,
    });
  }, [dispatch, key, scopeIdentity, accessGeneration]);

  // Reveal content only when explicitly requested.
  useEffect(() => {
    if (!item) return;
    const requested = revealRequest === `${item.store}|${item.path}`;
    if (!requested) return;
    onRevealHandled?.();
    // Reveal on a login shows its password, the field the list offers.
    const current = read?.key === key;
    dispatch({ type: 'show-rows', ids: ['password'], keep: current });
    if (!current) void show();
  }, [dispatch, item, key, onRevealHandled, read?.key, revealRequest, show]);

  // Revealed values and plaintext edit drafts are covered without abandoning
  // the details panel or its unsaved work.
  useConcealOnInactive(
    () => dispatch({ type: 'conceal-inactive', file: item?.kind === 'File' }),
    Boolean(read) || editing || saving,
  );

  // A read in flight when access changes generation is dropped when it lands,
  // so the control it disabled is released now instead of reading forever.
  useEffect(() => {
    dispatch({ type: 'generation-changed' });
  }, [dispatch, accessGeneration]);

  useEffect(() => {
    if (concealSignal) dispatch({ type: 'conceal-signal' });
  }, [dispatch, accessGeneration, concealSignal]);

  useEffect(() => {
    if (!item || !resumeDraft) return;
    if (item.store !== resumeDraft.store || item.path !== resumeDraft.path)
      return;
    // Restore the draft only once per refresh epoch to avoid overwriting newer changes.
    if (appliedDraft.current === resumeDraft.epoch) return;
    appliedDraft.current = resumeDraft.epoch;
    dispatch({
      type: 'restore-draft',
      value: resumeDraft.value,
      loginLines:
        kindOf(item) === 'Password' ? loginEditLines(resumeDraft.value) : null,
      binding: { item, scope: scopeIdentity, generation: accessGeneration },
    });
  }, [dispatch, item, resumeDraft, scopeIdentity, accessGeneration]);

  useEffect(() => {
    if (!editing || !focusEditorOnOpen.current) return;
    focusEditorOnOpen.current = false;
    editorRef.current
      ?.querySelector<HTMLElement>(
        'input:not([type="hidden"]), textarea, .eact button',
      )
      ?.focus();
  }, [editing]);

  useEffect(() => {
    if (editing || !focusActionsOnClose.current) return;
    focusActionsOnClose.current = false;
    // Back to Edit, the control that opened the editor.
    editorRef.current
      ?.querySelector<HTMLElement>('.detail-actions [data-edit-action]')
      ?.focus();
  }, [editing]);

  const selectedFileMode = item?.kind === 'File' || binaryFile;
  useFileDrop({
    bridge,
    active: editing && selectedFileMode,
    onHover: setDropHover,
    onPaths: (paths) => {
      setDropHover(false);
      dispatch(
        paths.length === 1
          ? { type: 'replacement', path: paths[0] ?? null, error: null }
          : {
              type: 'replacement',
              path: null,
              error: 'Drop exactly one file to replace the current contents.',
            },
      );
    },
    onError: (error) =>
      dispatch({
        type: 'edit-error',
        message: normalizeCommandError(error).message,
      }),
  });

  /* ------------------------------------------------------- unsaved edits -- */

  /** Takes the editor down, so a confirmed move lands on a clean panel. */
  const clearEdit = useCallback(
    () => dispatch({ type: 'close-editor' }),
    [dispatch],
  );

  // An open editor holding the stored value is not unsaved work; a chosen
  // replacement file is, since the file editor carries no text of its own.
  const editDirty =
    editor !== null &&
    (editor.replacementPath !== null ||
      editor.baseline === null ||
      editor.value !== editor.baseline);
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
      <aside ref={panelRef} className="details" aria-label="Details">
        <div className="dh">
          <span className="t" />
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
  const replacing = editing && fileMode;
  const inlineEditing = editing && !fileMode;
  const displayKind: FilterKind = kind;
  const team = store?.kind === 'team';
  const saveProblem =
    writeProblem ??
    (editorBound(editor, scopeIdentity, accessGeneration)
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
  const copyField = async (field: ItemField): Promise<void> => {
    if (!requireAccess()) return;
    await copyAndAnnounce({
      bridge,
      item,
      field,
      toasts,
      onError: onCommandError,
    });
  };
  // Showing one login row reads the item once; the text then stays in
  // `read` while any row is shown, and later rows reveal without a read.
  const showRow = (id: string): void => {
    const held = shownValue !== null || reading;
    dispatch({ type: 'show-rows', ids: [id], keep: held });
    if (!held) void show();
  };
  // Hiding the last shown row drops the text.
  const hideRow = (id: string): void =>
    dispatch(
      revealed.some((field) => field !== id)
        ? { type: 'hide-row', id }
        : { type: 'hide-value' },
    );

  const beginEdit = async (): Promise<void> => {
    if (!request || !requireAccess(true)) return;
    dispatch({ type: 'edit-error', message: null });
    const binding = {
      item,
      scope: scopeIdentity,
      generation: accessGeneration,
    };
    if (fileMode) {
      dispatch({ type: 'open-editor', value: '', loginLines: null, binding });
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
      const opened = response.value;
      dispatch({
        type: 'open-editor',
        value: opened,
        loginLines: kind === 'Password' ? loginEditLines(opened) : null,
        binding,
      });
    } catch (error) {
      if (!current()) return;
      const typed = normalizeCommandError(error);
      if (typed.code === 'not-text' && item.kind === 'Secret') {
        dispatch({ type: 'not-text' });
        dispatch({ type: 'open-editor', value: '', loginLines: null, binding });
      } else {
        onCommandError(error, item);
      }
    } finally {
      setSaving(false);
    }
  };

  const saveEdit = async (): Promise<void> => {
    if (saving || !requireAccess(true)) return;
    if (
      !request ||
      !editor ||
      !editorBound(editor, scopeIdentity, accessGeneration)
    ) {
      dispatch({
        type: 'edit-error',
        message: fileMode
          ? 'Access changed. Reopen Replace before continuing.'
          : 'Access changed. Reopen the editor before saving.',
      });
      return;
    }
    const target = editor.binding.item;
    dispatch({ type: 'edit-error', message: null });
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
      if (!fileMode) focusActionsOnClose.current = true;
      dispatch({ type: 'close-editor' });
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

  // One row per listed line, split by the rules Copy follows. Changing a row
  // rewrites only its line, keeping its separator; every other line is saved
  // as it was read.
  const editLoginLines = editor?.loginLines ?? null;
  const editLines = editLoginLines ? loginLines(editValue) : null;
  const editRows =
    editLines && editLoginLines
      ? editLoginLines.flatMap((index) => {
          const line = editLines[index];
          return line ? [{ ...line, index }] : [];
        })
      : null;
  // What a typed password should not contain: the item's name, which is the
  // site for a login, and the user name being edited.
  const editContext = editRows
    ? [
        item ? nameOf(item.path) : '',
        ...editRows
          .filter(({ key }) => loginFieldOf(key) === 'username')
          .map(({ value }) => value),
      ]
    : [];
  const updateEditField = (index: number, value: string): void => {
    if (!editLines) return;
    dispatch({
      type: 'edit',
      value: editLines
        .map(({ key, separator, raw }, candidate) => {
          if (candidate !== index) return raw;
          const line =
            key === null ? value : fieldLineText(key, separator, value);
          return raw.endsWith('\r') ? `${line}\r` : line;
        })
        .join('\n'),
    });
  };

  // A concealed draft shows one masked Contents row in either editor.
  const preview =
    inlineEditing && editor.concealed ? (
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
              onClick={() => dispatch({ type: 'unconceal-draft' })}
            >
              <Icon name="eye" />
            </button>
          }
        >
          {MASK}
        </InsetRow>
      </Inset>
    ) : inlineEditing && editRows ? (
      <Inset className="edit">
        {editRows.map(({ key: field, value, index }) => {
          const loginKey = loginFieldOf(field);
          const label =
            loginKey !== null
              ? fieldLabel(loginKey)
              : field !== null
                ? fieldLabel(field)
                : 'Value';
          // A row is a line, and editing a row never adds or removes one,
          // so a line keeps its input when its parse changes.
          return loginKey === 'password' ? (
            <PasswordInput
              key={index}
              label={label}
              value={value}
              onChange={(next) => updateEditField(index, next)}
              shown={editPasswordShown}
              onShownChange={(shown) => {
                if (!shown || requireAccess())
                  dispatch({ type: 'show-password', shown });
              }}
              showProblem={readProblem ?? undefined}
              showProblemId={accessDescriptionId}
              context={editContext}
            />
          ) : (
            <Field
              key={index}
              label={label}
              value={value}
              onChange={(next) => updateEditField(index, next)}
            />
          );
        })}
      </Inset>
    ) : inlineEditing ? (
      <Inset variant="preview">
        <textarea
          aria-label="Contents"
          value={editValue}
          onChange={(event) =>
            dispatch({ type: 'edit', value: event.target.value })
          }
        />
      </Inset>
    ) : kind === 'Password' && !fileMode ? (
      <Inset variant="preview">
        {loginRows(shownValue).map(({ id, label, field, value }) => {
          const visible = shownValue !== null && revealed.includes(id);
          const name = label.toLowerCase();
          const absent = shownValue !== null && field !== null && !value;
          return (
            <InsetRow
              key={id}
              label={label}
              className="rev"
              valueClass={visible ? 'value-text' : 'mask'}
              action={
                <>
                  {visible ? (
                    <button
                      title="Hide"
                      className="value-action"
                      aria-label={`Hide ${name}`}
                      type="button"
                      onClick={() => hideRow(id)}
                    >
                      <Icon name="eyeOff" />
                    </button>
                  ) : (
                    <button
                      className="value-action"
                      aria-label={reading ? 'Reading…' : `Show ${name}`}
                      type="button"
                      disabled={reading || Boolean(readProblem)}
                      title={readProblem ?? (reading ? 'Reading…' : 'Show')}
                      aria-describedby={
                        readProblem ? accessDescriptionId : undefined
                      }
                      onClick={() => showRow(id)}
                    >
                      <Icon name="eye" />
                    </button>
                  )}
                  {field ? (
                    <button
                      className="value-action"
                      aria-label={COPY_LABELS[field]}
                      type="button"
                      disabled={Boolean(readProblem) || absent}
                      title={
                        readProblem ??
                        (absent ? `No ${name} saved` : COPY_LABELS[field])
                      }
                      aria-describedby={
                        readProblem ? accessDescriptionId : undefined
                      }
                      onClick={() => void copyField(field)}
                    >
                      <Icon name="copy" />
                    </button>
                  ) : null}
                </>
              }
            >
              {!visible ? (
                MASK
              ) : value ? (
                value
              ) : (
                <span className="dim">Not set</span>
              )}
            </InsetRow>
          );
        })}
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
                  onClick={() => void copyField('value')}
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
                  onClick={() => dispatch({ type: 'hide-value' })}
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
                  onClick={() => void copyField('value')}
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
            <small>
              {item.size === null
                ? 'Encrypted'
                : `${fmtSize(item.size)} · encrypted at rest`}
            </small>
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
  const itemRef = itemRefOf(item);
  const favorite = isFavorite(recentFavorites, itemRef);

  return (
    <>
      <aside
        ref={panelRef}
        className="details"
        aria-label={`Details for ${nameOf(item.path)}`}
      >
        <div className="dh">
          <KindIcon kind={displayKind} />
          <span className="t">
            <h2>{nameOf(item.path)}</h2>
            <small title={displayPath(item.path)}>{where}</small>
          </span>
          {itemRef ? (
            <button
              type="button"
              className={favorite ? 'x fav on' : 'x fav'}
              title={favorite ? 'Remove from favorites' : 'Add to favorites'}
              aria-label="Favorite"
              aria-pressed={favorite}
              onClick={() => toggleFavorite(itemRef)}
            >
              <Icon name="star" />
            </button>
          ) : null}
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
        <div className="scroll" ref={editorRef}>
          {blockingReason && !replacing ? (
            <p id={accessDescriptionId} className="action-error" role="status">
              {blockingReason}
            </p>
          ) : null}
          {/* Item actions; value controls live beside the value. */}
          {inlineEditing ? null : (
            <div className="detail-actions">
              {fileMode ? (
                <Button
                  icon="download"
                  disabled={Boolean(readProblem)}
                  title={readProblem ?? 'Save a copy to disk'}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={downloadFile}
                >
                  Download
                </Button>
              ) : null}
              {displayKind === 'Password' && !fileMode ? (
                <Button
                  variant="primary"
                  icon="copy"
                  disabled={Boolean(readProblem)}
                  title={readProblem ?? 'Copy only the password'}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={() => void copyField('password')}
                >
                  {COPY_LABELS.password}
                </Button>
              ) : null}
              {displayKind === 'Password' &&
              !fileMode &&
              item.shape === undefined ? (
                // Known as a Password by its path alone, the text may hold
                // no `password:` line; this copies it whole.
                <Button
                  icon="copy"
                  disabled={Boolean(readProblem)}
                  title={readProblem ?? 'Copy the whole text'}
                  aria-describedby={
                    readProblem ? accessDescriptionId : undefined
                  }
                  onClick={() => void copyField('value')}
                >
                  {COPY_LABELS.value}
                </Button>
              ) : null}
              <Button
                data-edit-action
                icon={fileMode ? 'refresh' : 'pencil'}
                disabled={Boolean(writeProblem) || saving}
                title={
                  writeProblem ??
                  (fileMode ? 'Replace this file' : 'Edit this item')
                }
                aria-describedby={
                  writeProblem ? accessDescriptionId : undefined
                }
                onClick={() => {
                  focusEditorOnOpen.current = true;
                  void beginEdit();
                }}
              >
                {saving && !fileMode
                  ? 'Reading…'
                  : fileMode
                    ? 'Replace'
                    : 'Edit'}
              </Button>
              {onMove ? (
                <Button
                  disabled={Boolean(writeProblem)}
                  title={writeProblem}
                  onClick={() => {
                    if (requireAccess(true)) onMove(item);
                  }}
                >
                  Rename or move…
                </Button>
              ) : null}
              {onCopyToVault ? (
                <Button
                  disabled={Boolean(readProblem ?? copyProblem(item))}
                  title={
                    readProblem ??
                    copyProblem(item) ??
                    'Copy this item into another vault'
                  }
                  onClick={() => {
                    if (requireAccess()) onCopyToVault(item);
                  }}
                >
                  Copy to vault…
                </Button>
              ) : null}
              <Button
                variant="danger"
                icon="trash"
                disabled={Boolean(writeProblem)}
                title={writeProblem ?? 'Move this item to Trash'}
                aria-describedby={
                  writeProblem ? accessDescriptionId : undefined
                }
                onClick={() => {
                  if (requireAccess(true)) onTrash(item);
                }}
              >
                Move to Trash
              </Button>
            </div>
          )}
          {inlineEditing || displayKind === 'Password' ? (
            <SectionLabel>{inlineEditing ? 'Edit' : 'Fields'}</SectionLabel>
          ) : null}
          {preview}
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
          {inlineEditing ? (
            <div className="eact">
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
                aria-describedby={saveProblem ? accessDescriptionId : undefined}
                onClick={() => void saveEdit()}
              >
                {saving ? 'Saving…' : 'Save changes'}
              </Button>
            </div>
          ) : null}

          <Inset>
            <InsetRow label="Kind">{kindLabel(displayKind)}</InsetRow>
            <InsetRow label="Location" valueClass="mark">
              {store ? (
                <StoreMark
                  snapshot={snapshot}
                  store={store}
                  hue={storeHues(storeNavigationOrder(snapshot)).get(store.id)}
                />
              ) : null}
              <span title={displayPath(item.path)}>{where}</span>
            </InsetRow>
            <InsetRow label="Path">{displayPath(item.path)}</InsetRow>
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
                    variant="listing"
                    party={party}
                    caption={[
                      party.party_kind !== 'user' ? 'Member team' : '',
                      readers.includes(party) ? '' : 'No read access',
                    ]
                      .filter(Boolean)
                      .join(' · ')}
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
      {replacing ? (
        <SheetDialog
          title={`Replace ${nameOf(item.path)}`}
          glyph={<KindIcon kind={displayKind} />}
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
                aria-describedby={saveProblem ? accessDescriptionId : undefined}
                onClick={() => void saveEdit()}
              >
                {saving
                  ? 'Replacing…'
                  : replacementPath
                    ? 'Replace file'
                    : 'Choose file…'}
              </Button>
            </>
          }
        >
          <p>
            Choose a file to replace the contents of <b>{nameOf(item.path)}</b>.
          </p>
          <Inset
            variant="preview"
            className={dropHover ? 'drop-hover' : undefined}
          >
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
