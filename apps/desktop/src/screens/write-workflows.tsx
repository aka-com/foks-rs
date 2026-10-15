import { MoveSheet } from './move-item-sheet';
import { BulkProgressSheet } from './bulk-progress-sheet';
import { useTabSheetState } from '../navigation-guard';
import { useCallback, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Dialog, DismissibleDialog } from '../../kit/overlay-primitives';
import {
  Band,
  Button,
  CardSelect,
  DocumentSourceRow,
  DocumentSourceSwitch,
  Field,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  PasswordInput,
  SectionLabel,
  Sheet,
  SheetDialog,
} from '../components';
import type { DocumentSourceKind, FilterKind } from '../components';
import { useConcealOnInactive } from '../use-conceal-on-inactive';
import {
  displayPath,
  isLogin,
  itemKey,
  kindLabel,
  kindOf,
  canCreateInStore,
  defaultCreateStore,
  loginField,
  loginFieldOf,
  loginLines,
  nameOf,
  parseTrashName,
  restoredCopyPath,
  storeNavigationOrder,
  storeAvailability,
  storeOf,
  trashEntries,
  trashEntryLabel,
} from '../model';
import type {
  Item,
  ItemKind,
  LoginFieldName,
  StoreRef,
  AgentSnapshot,
} from '../model';
import { normalizeCommandError } from '../bridge';
import type { KvRoleInput } from '../bridge';
import type { GuardVerdict } from '../location';
import { NavigationPrompt, useSheetGuard } from '../navigation-guard';
import { useFileDrop } from '../file-drop';
import { useToast } from '../../kit/toasts';
import { itemActionProblem } from './store-access';
import { useTrashOrigins } from './trash-origins';
import {
  DEFAULT_READ_ROLE,
  DEFAULT_WRITE_ROLE,
  newFolderRoles,
} from './item-roles';
import { AccessBlock, storeOption, writeBlockReason } from './store-choice';
import { CopyItemSheet } from './copy-item-sheet';
import { ImportPasswordsSheet } from './import-passwords-sheet';
import { ExportPasswordsSheet } from './export-passwords-sheet';
import { useShellServices, useShellSnapshot } from '../shell-services';

export { writeBlockReason };

export type NewKind = Exclude<ItemKind, 'Folder'>;

/** Default item kind used when opening the new-item sheet without one. */
export const DEFAULT_NEW_KIND: NewKind = 'Password';

/** Refusal shown when navigating while a save request is in flight. */
const SAVING_REFUSAL = 'Wait for the item to finish saving.';

/**
 * Returns confirmation options for discarding an unsaved draft, shared by
 * navigation guards, Escape handling, and backdrop dismissal.
 */
function discardNewItem(
  itemKind: NewKind,
  onConfirm?: () => void,
): Extract<GuardVerdict, { verdict: 'prompt' }> {
  return {
    verdict: 'prompt',
    title: 'Discard new item?',
    body: `Your new ${kindLabel(itemKind).toLowerCase()} has not been saved.`,
    confirm: 'Discard',
    ...(onConfirm ? { onConfirm } : {}),
  };
}

export { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE };

/** A partially filled new-item form, carried across conflict recovery. */
export interface NewDraft {
  path: string;
  site: string;
  username: string;
  password: string;
  website: string;
  value: string;
  resourceName: string;
  sourcePath: string | null;
  readRole: KvRoleInput;
  writeRole: KvRoleInput;
}

export type WriteWorkflow =
  | {
      kind: 'new';
      itemKind: NewKind;
      storeId: StoreRef;
      initialFolder?: string;
      draft?: NewDraft;
    }
  | {
      kind: 'new-folder';
      storeId: StoreRef;
      initialFolder?: string;
    }
  | {
      kind: 'exists';
      itemKind: NewKind;
      storeId: StoreRef;
      path: string;
      initialFolder?: string;
      draft?: NewDraft;
    }
  | { kind: 'move'; item: Item }
  /** Copies an item into another vault, optionally moving the original to Trash. */
  | { kind: 'copy'; item: Item }
  /**
   * Imports logins from another password manager's CSV export, into
   * `storeId` unless another vault is chosen.
   */
  | { kind: 'import'; storeId: StoreRef; initialFolder?: string }
  /** Writes the passwords of `storeId` to a plain-text CSV file. */
  | { kind: 'export'; storeId: StoreRef }
  /** Several items of one store, moved together to a folder chosen next. */
  | { kind: 'move-many'; items: readonly Item[] }
  /** Moves `items` into `folder` of their store one at a time, with progress. */
  | { kind: 'bulk-move'; items: readonly Item[]; folder: string }
  /** Moves `items` to their stores' Trash one at a time, with progress. */
  | { kind: 'bulk-trash'; items: readonly Item[] }
  /**
   * Moves Trash entries back to where `items` were, one at a time, with
   * progress: the Undo of a bulk-trash run. `trash[i]` is the path in Trash
   * of `items[i]`.
   */
  | {
      kind: 'bulk-restore';
      items: readonly Item[];
      trash: readonly string[];
    }
  /**
   * Permanently removes an entry in Trash, or with `outsideTrash` an item in
   * a team whose Trash this account cannot set up.
   */
  | { kind: 'delete'; item: Item; outsideTrash?: boolean }
  /** Restoring a Trash entry found another item at `destination`. */
  | { kind: 'restore-conflict'; item: Item; destination: string }
  | { kind: 'empty-trash'; store: StoreRef }
  | {
      kind: 'conflict';
      item: Item;
      draft: string;
      operation?: 'edit' | 'replace';
    }
  | null;

/** What the Path field shows while it is empty. */
const PATH_HINT: Readonly<Record<NewKind, string>> = {
  Password: '/logins/github.com',
  Document: '/passport-scan.pdf',
};

/** The vault path a dropped file takes: the current folder, else the root. */
export function fileDropPath(name: string, folder?: string): string {
  return namedPath(name, '/', folder);
}

function namedPath(
  name: string,
  defaultDirectory: string,
  folder?: string,
): string {
  const directory = (
    folder && folder !== '/' ? folder : defaultDirectory
  ).replace(/\/$/, '');
  return `${directory}/${name}`;
}

/**
 * The new-item draft a dropped file resumes from, so a drop that lands on an
 * occupied path can reopen the sheet with the file and path already chosen.
 */
export function droppedFileDraft(path: string, sourcePath: string): NewDraft {
  return {
    path,
    site: '',
    username: '',
    password: '',
    website: '',
    value: '',
    resourceName: '',
    sourcePath,
    readRole: DEFAULT_READ_ROLE,
    writeRole: DEFAULT_WRITE_ROLE,
  };
}

/**
 * Whether the New password sheet holds all of this text: every line that is
 * not empty names a different one of its three fields.
 */
function fitsPasswordSheet(text: string): boolean {
  const named = new Set<LoginFieldName>();
  return loginLines(text).every(({ key, raw }) => {
    if (key === null && (raw === '' || raw === '\r')) return true;
    const field = loginFieldOf(key);
    if (field === null || named.has(field)) return false;
    named.add(field);
    return true;
  });
}

/**
 * The new-item sheet an edit resumes in when the item it was editing has been
 * deleted elsewhere: there is no newer version to review, so the edit is
 * offered as a new item at the same path, for the user to save or discard.
 * A login resumes in the Password sheet only when that sheet's fields hold
 * the whole draft; otherwise it resumes as a Document with the draft as its
 * value, so no line is dropped.
 */
export function conflictDraftWorkflow(
  item: Item,
  draft: string,
): WriteWorkflow {
  const itemKind: NewKind =
    kindOf(item) === 'Password' && fitsPasswordSheet(draft)
      ? 'Password'
      : 'Document';
  const field = (name: LoginFieldName): string =>
    itemKind === 'Password' ? (loginField(draft, name) ?? '') : '';
  return {
    kind: 'new',
    itemKind,
    storeId: item.store,
    draft: {
      path: item.path,
      site: itemKind === 'Password' ? nameOf(item.path) : '',
      username: field('username'),
      password: field('password'),
      website: field('url'),
      value: itemKind === 'Password' ? '' : draft,
      resourceName: '',
      sourcePath: null,
      readRole: DEFAULT_READ_ROLE,
      writeRole: DEFAULT_WRITE_ROLE,
    },
  };
}

export function initialWriteWorkflow(
  search: string,
  snapshot?: AgentSnapshot,
): WriteWorkflow {
  const state = new URLSearchParams(search).get('state');
  if (state === 'new')
    return { kind: 'new', itemKind: 'Password', storeId: 'team:household' };
  if (state === 'new-group')
    return { kind: 'new', itemKind: 'Document', storeId: 'team:eng' };
  if (state === 'new-document')
    return { kind: 'new', itemKind: 'Document', storeId: 'acct:personal' };
  if (state === 'group-new-document')
    return { kind: 'new', itemKind: 'Document', storeId: 'team:eng' };
  if (state === 'exists') {
    return {
      kind: 'exists',
      itemKind: 'Password',
      storeId: 'acct:personal',
      path: PATH_HINT.Password,
      initialFolder: '/logins',
    };
  }
  if (state === 'conflict' && snapshot) {
    const item = snapshot.items.find((candidate) => isLogin(candidate));
    if (item)
      return {
        kind: 'conflict',
        item,
        draft: snapshot.plaintext[itemKey(item)] ?? '',
      };
  }
  return null;
}

export function workflowForError(
  error: unknown,
  item?: Item,
  draft = '',
): WriteWorkflow | null {
  const typed = normalizeCommandError(error);
  if (typed.code === 'conflict' && item)
    return { kind: 'conflict', item, draft };
  return null;
}

interface NewSheetProps {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}

function NewSheet({ workflow, setWorkflow }: NewSheetProps): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    commandError: onError,
    mutationError: onMutationError,
    accessNow,
  } = useShellServices();
  // If the specified store is not available locally, fall back to the default vault.
  const [storeId, setStoreId] = useTabSheetState('item.storeId', () =>
    storeOf(snapshot, workflow.storeId)
      ? workflow.storeId
      : (defaultCreateStore(snapshot) ?? workflow.storeId),
  );
  const [site, setSite] = useTabSheetState(
    'item.site',
    workflow.draft?.site ?? '',
  );
  const [path, setPath] = useTabSheetState(
    'item.path',
    workflow.draft?.path ??
      (workflow.itemKind === 'Password'
        ? namedPath(
            workflow.draft?.site ?? '',
            '/logins',
            workflow.initialFolder,
          )
        : namedPath('', '/', workflow.initialFolder)),
  );
  const [pathShown, setPathShown] = useState(false);
  const [username, setUsername] = useTabSheetState(
    'item.username',
    workflow.draft?.username ?? '',
  );
  const [password, setPassword] = useTabSheetState(
    'item.password',
    workflow.draft?.password ?? '',
  );
  const [passwordShown, setPasswordShown] = useState(false);
  const [website, setWebsite] = useTabSheetState(
    'item.website',
    workflow.draft?.website ?? '',
  );
  const [value, setValue] = useTabSheetState(
    'item.value',
    workflow.draft?.value ?? '',
  );
  const [valueConcealed, setValueConcealed] = useState(false);
  const [resourceName, setResourceName] = useTabSheetState(
    'item.resourceName',
    workflow.draft?.resourceName ?? '',
  );
  const [sourcePath, setSourcePath] = useTabSheetState<string | null>(
    'item.sourcePath',
    workflow.draft?.sourcePath ?? null,
  ); // A document is typed text or an imported file; the switch picks which, and
  // dropping a file selects the File option automatically.
  const [source, setSource] = useTabSheetState<DocumentSourceKind>(
    'item.source',
    workflow.draft?.sourcePath ? 'file' : 'text',
  );
  const [readRole, setReadRole] = useTabSheetState<KvRoleInput>(
    'item.readRole',
    workflow.draft?.readRole ?? DEFAULT_READ_ROLE,
  );
  const [writeRole, setWriteRole] = useTabSheetState<KvRoleInput>(
    'item.writeRole',
    workflow.draft?.writeRole ?? DEFAULT_WRITE_ROLE,
  );
  const [hovering, setHovering] = useState(false);
  const [saving, setSaving] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  const store = storeOf(snapshot, storeId);
  const group = store?.kind === 'team';
  const canWrite = Boolean(store && canCreateInStore(snapshot, store.id));
  const blocked = store ? writeBlockReason(snapshot, store) : null;
  const itemKind = workflow.itemKind;
  useConcealOnInactive(
    () => setValueConcealed(true),
    itemKind === 'Document' && source === 'text' && Boolean(value),
  );
  useConcealOnInactive(
    () => setPasswordShown(false),
    itemKind === 'Password' && passwordShown,
  );
  const roleArgs = group ? { readRole, writeRole } : {};

  useFileDrop({
    bridge,
    active: itemKind === 'Document',
    onHover: setHovering,
    onPaths: (paths) => {
      setHovering(false);
      if (paths.length !== 1) {
        if (sourcePath)
          void bridge.releaseImportFile(sourcePath).catch(onError);
        setSourcePath(null);
        setFileError('Drop exactly one file.');
        return;
      }
      const first = paths[0];
      if (!first) return;
      if (sourcePath && sourcePath !== first)
        void bridge.releaseImportFile(sourcePath).catch(onError);
      setSourcePath(first);
      setSource('file');
      setFileError(null);
      const name = first.split(/[\\/]/).at(-1);
      if (name) setPath(fileDropPath(name, workflow.initialFolder));
    },
    onError: (error) => setFileError(normalizeCommandError(error).message),
  });

  /* --------------------------------------------------- unsaved new item -- */

  // Track the initial path to detect unsaved changes. Site and Name fields
  // account for path changes, and changing the item kind alone does not create
  // unsaved content.
  const openedPath = useRef(path).current;
  const hasContent =
    Boolean(
      site ||
      username ||
      password ||
      website ||
      value ||
      resourceName ||
      sourcePath,
    ) || path !== openedPath;
  const close = useCallback(() => {
    if (sourcePath) void bridge.releaseImportFile(sourcePath).catch(onError);
    setWorkflow(null);
  }, [bridge, onError, setWorkflow, sourcePath]);
  // Save requests in flight cannot be aborted. Pending drafts require user
  // confirmation before any action that unmounts the sheet.
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: SAVING_REFUSAL }
      : hasContent
        ? discardNewItem(itemKind, close)
        : null,
    !saving,
  );
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const toasts = useToast();
  // Escape and the backdrop are answered the same way a navigation is.
  const dismiss = (): void => {
    if (saving) {
      toasts.show(SAVING_REFUSAL, { tone: 'warning' });
      return;
    }
    if (hasContent) {
      setConfirmingDiscard(true);
      return;
    }
    setWorkflow(null);
  };

  const pathName = path.split('/').at(-1) ?? '';
  const submit = async (): Promise<void> => {
    if (
      !store ||
      !canWrite ||
      !storeAvailability(snapshot, store, { nowSeconds: accessNow() })
        .available ||
      !canCreateInStore(snapshot, store.id) ||
      saving ||
      !path.startsWith('/') ||
      !pathName
    )
      return;
    setSaving(true);
    let uploadSourcePath = sourcePath;
    let created = false;
    try {
      if (itemKind === 'Password') {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value: `user: ${username}\npassword: ${password}\nurl: ${website}`,
          ...roleArgs,
        });
      } else if (source === 'file') {
        if (!uploadSourcePath) {
          uploadSourcePath = await bridge.pickImportFile();
          if (!uploadSourcePath) return;
          setSourcePath(uploadSourcePath);
          setFileError(null);
        }
        await bridge.importDroppedFile({
          storeId: store.id,
          path,
          sourcePath: uploadSourcePath,
          ...roleArgs,
        });
      } else {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value,
          ...roleArgs,
        });
      }
      created = true;
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'already-exists') {
        setWorkflow({
          kind: 'exists',
          itemKind,
          storeId: store.id,
          path,
          initialFolder: path.slice(0, path.lastIndexOf('/')) || '/',
          draft: {
            path,
            site,
            username,
            password,
            website,
            value,
            resourceName,
            sourcePath: uploadSourcePath,
            readRole,
            writeRole,
          },
        });
        await onMutationError(error, { report: false });
      } else if (typed.code === 'inactive-group') {
        try {
          await onApplied(
            `Cannot create item: ${store.name} is inactive`,
            store.server,
          );
          setWorkflow(null);
        } catch (refreshError) {
          onError(refreshError);
        }
      } else await onMutationError(error);
    } finally {
      setSaving(false);
    }
    if (!created) return;
    setWorkflow(null);
    try {
      await onApplied(
        `${kindLabel(itemKind)} created in ${store.name}`,
        store.server,
      );
    } catch (error) {
      onError(error);
    }
  };

  const field = (
    label: string,
    current: string,
    set: (next: string) => void,
    placeholder: string,
    mono = false,
  ) => (
    <Field
      label={label}
      value={current}
      onChange={set}
      placeholder={placeholder}
      mono={mono}
    />
  );
  const sheet = (
    <Sheet
      width="mid"
      glyph={<KindIcon kind={itemKind} />}
      title={`New ${kindLabel(itemKind).toLowerCase()}`}
      footer={
        <>
          <Button onClick={dismiss}>Cancel</Button>
          <Button
            variant="primary"
            disabled={
              !canWrite ||
              saving ||
              !pathName ||
              (itemKind === 'Document' && source === 'text' && !value.trim())
            }
            onClick={() => void submit()}
          >
            {saving
              ? 'Creating…'
              : itemKind === 'Document' && source === 'file' && !sourcePath
                ? 'Choose file and create'
                : 'Create item'}
          </Button>
        </>
      }
    >
      <>
        <SectionLabel
          action={
            snapshot.stores.length ? (
              <button
                type="button"
                className="path-toggle"
                aria-expanded={pathShown}
                onClick={() => setPathShown((shown) => !shown)}
              >
                {pathShown ? 'Hide Path' : 'Path'}
              </button>
            ) : undefined
          }
        >
          Save in vault
        </SectionLabel>
        <Inset>
          {snapshot.stores.length ? (
            <CardSelect
              label="Save in vault"
              options={storeNavigationOrder(snapshot).map((candidate) =>
                storeOption(snapshot, candidate),
              )}
              value={storeId}
              onChange={setStoreId}
              placeholder="Choose a destination"
            />
          ) : (
            <InsetRow label="Vault">
              <span className="dim">No vaults available to store items.</span>
            </InsetRow>
          )}
          {snapshot.stores.length && pathShown
            ? field('Path', path, setPath, PATH_HINT[itemKind])
            : null}
        </Inset>
        {store && blocked ? (
          <Band live>{`${store.name}: ${blocked}`}</Band>
        ) : null}
        <SectionLabel
          action={
            itemKind === 'Document' ? (
              <DocumentSourceSwitch value={source} onChange={setSource} />
            ) : undefined
          }
        >
          {kindLabel(itemKind)}
        </SectionLabel>
        <Inset
          className={
            itemKind === 'Document' && hovering ? 'drop-hover' : undefined
          }
        >
          {itemKind === 'Password' ? (
            <>
              {field(
                'Site',
                site,
                (next) => {
                  setSite(next);
                  setPath(namedPath(next, '/logins', workflow.initialFolder));
                },
                'e.g. github.com',
              )}
              {field('User name', username, setUsername, 'username')}
              <PasswordInput
                value={password}
                onChange={setPassword}
                shown={passwordShown}
                onShownChange={setPasswordShown}
                context={[site, username]}
              />
              {field(
                'Website',
                website,
                setWebsite,
                'https://github.com/login',
              )}
            </>
          ) : null}
          {itemKind === 'Document' ? (
            <>
              {field(
                'Name',
                resourceName,
                (next) => {
                  setResourceName(next);
                  if (next)
                    setPath(
                      namedPath(
                        next.toLowerCase().replace(/[^a-z0-9._-]+/g, '-'),
                        '/',
                        workflow.initialFolder,
                      ),
                    );
                },
                'e.g. api-key',
              )}
              <DocumentSourceRow
                source={source}
                value={value}
                onValue={setValue}
                sourcePath={sourcePath}
                hovering={hovering}
                concealed={valueConcealed}
                onReveal={() => setValueConcealed(false)}
              />
            </>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
        {store ? (
          <AccessBlock
            snapshot={snapshot}
            store={store}
            readRole={readRole}
            writeRole={writeRole}
            onReadRole={setReadRole}
            onWriteRole={setWriteRole}
          />
        ) : null}
      </>
    </Sheet>
  );
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label={`New ${kindLabel(itemKind).toLowerCase()}`}
        onDismiss={dismiss}
      >
        {sheet}
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discardNewItem(itemKind)}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}

function NewFolderSheet({
  workflow,
  setWorkflow,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new-folder' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
    accessNow,
  } = useShellServices();
  const [name, setName] = useTabSheetState('folder.name', '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const store = storeOf(snapshot, workflow.storeId);
  const blocked = store ? writeBlockReason(snapshot, store) : null;
  const validName = Boolean(
    name.trim() &&
    name.trim() !== '.' &&
    name.trim() !== '..' &&
    !/[\\/]/.test(name),
  );
  const canWrite = Boolean(
    store &&
    !blocked &&
    storeAvailability(snapshot, store, { nowSeconds: accessNow() }).available &&
    canCreateInStore(snapshot, store.id),
  );
  const close = useCallback(() => setWorkflow(null), [setWorkflow]);
  const discard = (): Extract<GuardVerdict, { verdict: 'prompt' }> => ({
    verdict: 'prompt',
    title: 'Discard new folder?',
    body: 'The folder has not been created.',
    confirm: 'Discard',
  });
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: SAVING_REFUSAL }
      : name
        ? { ...discard(), onConfirm: close }
        : null,
    !saving,
  );
  const dismiss = () => {
    if (saving) return;
    if (name) setConfirmingDiscard(true);
    else close();
  };
  const submit = async () => {
    if (!store || !canWrite || !validName || saving) return;
    setSaving(true);
    setError(null);
    try {
      await bridge.createFolder({
        storeId: store.id,
        path: namedPath(name.trim(), '/', workflow.initialFolder),
        ...newFolderRoles(store),
      });
      await onApplied(`Folder created in ${store.name}`, store.server);
      close();
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      await onMutationError(failure, { report: false });
    } finally {
      setSaving(false);
    }
  };
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label="New folder"
        onDismiss={dismiss}
      >
        <Sheet
          width="mid"
          glyph={
            <span className="kic Folder" aria-hidden="true">
              <Icon name="folder" />
            </span>
          }
          title="New folder"
          footer={
            <>
              <Button onClick={dismiss}>Cancel</Button>
              <Button
                variant="primary"
                disabled={!canWrite || !validName || saving}
                onClick={() => void submit()}
              >
                {saving ? 'Creating…' : 'Create folder'}
              </Button>
            </>
          }
        >
          <SectionLabel>Location</SectionLabel>
          <Inset>
            <InsetRow label="Vault">
              <span>{store?.name ?? 'Vault unavailable'}</span>
            </InsetRow>
            {workflow.initialFolder && workflow.initialFolder !== '/' ? (
              <InsetRow label="Folder">
                <span>{workflow.initialFolder}</span>
              </InsetRow>
            ) : null}
            <Field
              label="Name"
              value={name}
              onChange={(next) => {
                setName(next);
                setError(null);
              }}
              placeholder="Folder name"
            />
          </Inset>
          {name && !validName ? (
            <p className="action-error" role="alert">
              Enter a name without slashes, “.”, or “..”.
            </p>
          ) : null}
          {store && blocked ? (
            <Band live>{`${store.name}: ${blocked}`}</Band>
          ) : null}
          {error ? (
            <p className="action-error" role="alert">
              {error}
            </p>
          ) : null}
        </Sheet>
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discard()}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}

function ExistsSheet({
  workflow,
  setWorkflow,
  onOpenExisting,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const { bridge, commandError: onError } = useShellServices();
  const clash = snapshot.items.find(
    (item) => item.store === workflow.storeId && item.path === workflow.path,
  );
  // This step still holds the draft the new-item sheet carried here, so it is
  // abandoned under the same confirmation, from a navigation or from Escape
  // and the backdrop.
  const close = useCallback(() => {
    const sourcePath = workflow.draft?.sourcePath;
    if (sourcePath) void bridge.releaseImportFile(sourcePath).catch(onError);
    setWorkflow(null);
  }, [bridge, onError, setWorkflow, workflow.draft?.sourcePath]);
  useSheetGuard(discardNewItem(workflow.itemKind, close));
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const sheet = (
    <Sheet
      glyph={
        clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : undefined
      }
      title={`An item already exists at ${displayPath(workflow.path)}`}
      footer={
        <>
          <Button
            onClick={() =>
              setWorkflow({
                kind: 'new',
                itemKind: workflow.itemKind,
                storeId: workflow.storeId,
                initialFolder: workflow.initialFolder,
                draft: workflow.draft,
              })
            }
          >
            Change path
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              void onOpenExisting(workflow).then(close, onError);
            }}
          >
            Open existing item
          </Button>
        </>
      }
    >
      <>
        <p>
          The upload was skipped. Open the existing item and replace it, or
          choose another path.
        </p>
      </>
    </Sheet>
  );
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label="Item path already exists"
        onDismiss={() => setConfirmingDiscard(true)}
      >
        {sheet}
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discardNewItem(workflow.itemKind)}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}

export function WriteOverlay({
  workflow,
  setWorkflow,
  onRefreshConflict,
  onDiscardConflict,
  onDeleteConflict,
  onOpenExisting,
  onRestoreTrash,
}: {
  workflow: WriteWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
  /**
   * Refreshes after an edit conflict and says what the conflict sheet gives
   * way to: nothing, when the item is there to review, or a new-item sheet
   * carrying the edit when the item turned out to have been deleted.
   */
  onRefreshConflict: (
    item: Item,
    draft: string,
    operation?: 'edit' | 'replace',
  ) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
  /** Refreshes after a delete conflict and states whether the item is still there. */
  onDeleteConflict: (item: Item) => Promise<void>;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  /** Moves a Trash entry to `destination`, outside Trash. */
  onRestoreTrash: (item: Item, destination: string) => Promise<void>;
}): ReactNode {
  if (!workflow) return null;
  // The sheet supplies its own dialog: Escape and the backdrop are decided by
  // the draft it holds, the same state its navigation guard answers from.
  if (workflow.kind === 'new')
    return <NewSheet workflow={workflow} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'new-folder')
    return <NewFolderSheet workflow={workflow} setWorkflow={setWorkflow} />;
  // The already-exists step carries the same draft, so it supplies its own
  // dialog for the same reason.
  if (workflow.kind === 'exists')
    return (
      <ExistsSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onOpenExisting={onOpenExisting}
      />
    );
  if (workflow.kind === 'move' || workflow.kind === 'move-many')
    return (
      <MoveSheet
        items={workflow.kind === 'move' ? [workflow.item] : workflow.items}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'copy')
    return <CopyItemSheet item={workflow.item} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'export')
    return (
      <ExportPasswordsSheet
        storeId={workflow.storeId}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'import')
    return (
      <ImportPasswordsSheet workflow={workflow} setWorkflow={setWorkflow} />
    );
  if (
    workflow.kind === 'bulk-move' ||
    workflow.kind === 'bulk-trash' ||
    workflow.kind === 'bulk-restore'
  )
    return (
      // The Undo of a bulk-trash run replaces it with a run of its own.
      <BulkProgressSheet
        key={workflow.kind}
        workflow={workflow}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'restore-conflict')
    return (
      <RestoreConflictSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onRestoreTrash={onRestoreTrash}
      />
    );
  if (workflow.kind === 'empty-trash')
    return <EmptyTrashSheet workflow={workflow} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'conflict')
    return (
      <ConflictSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onRefreshConflict={onRefreshConflict}
        onDiscardConflict={onDiscardConflict}
      />
    );
  return (
    <DeleteSheet
      workflow={workflow}
      setWorkflow={setWorkflow}
      onDeleteConflict={onDeleteConflict}
    />
  );
}

/**
 * Modal sheet displayed when a save conflict occurs, prompting the user to review or discard changes.
 */
function ConflictSheet({
  workflow,
  setWorkflow,
  onRefreshConflict,
  onDiscardConflict,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRefreshConflict: (
    item: Item,
    draft: string,
    operation?: 'edit' | 'replace',
  ) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
}): ReactNode {
  const { commandError: onError } = useShellServices();
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const replacing = workflow.operation === 'replace';

  if (confirmingDiscard)
    return (
      <Dialog
        className="backdrop"
        role="alertdialog"
        aria-label="Discard unsaved changes"
        onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
          // Escape closes the confirmation dialog without discarding the draft.
          if (event.key === 'Escape') setConfirmingDiscard(false);
        }}
      >
        <Sheet
          glyph={
            <span className="kico md danger">
              <Icon name="trash" />
            </span>
          }
          title="Discard unsaved changes?"
          footer={
            <>
              <Button onClick={() => setConfirmingDiscard(false)}>
                {replacing ? 'Keep replacing' : 'Keep editing'}
              </Button>
              <Button variant="primary" danger onClick={onDiscardConflict}>
                Discard changes
              </Button>
            </>
          }
        >
          <p>Your changes have not been saved.</p>
        </Sheet>
      </Dialog>
    );

  return (
    <Dialog
      className="backdrop"
      aria-label={replacing ? 'Replace conflict' : 'Edit conflict'}
      onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === 'Escape') setConfirmingDiscard(true);
      }}
    >
      <Sheet
        glyph={<KindIcon kind={kindOf(workflow.item) as FilterKind} />}
        title="Item changed elsewhere"
        footer={
          <>
            <Button onClick={() => setConfirmingDiscard(true)}>
              {replacing ? 'Discard replacement' : 'Discard my edit'}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                // The refresh finds out whether the item was changed or
                // removed, and says what this sheet gives way to.
                void onRefreshConflict(
                  workflow.item,
                  workflow.draft,
                  workflow.operation,
                ).then(
                  (next) => setWorkflow(next),
                  (error) => onError(error),
                );
              }}
            >
              Refresh and review
            </Button>
          </>
        }
      >
        <>
          <p>
            {/* The agent reports one conflict for an item that was changed and
                one that was removed, so this does not claim to know which. */}
            {replacing
              ? 'This file changed or was removed while you were replacing it. Refresh to review the current item, then choose the replacement again.'
              : 'This item was changed or removed while you were editing. Refresh to see what happened; your edit is kept until you discard it.'}
          </p>
        </>
      </Sheet>
    </Dialog>
  );
}

/** What permanent deletion says it does, in every sheet that offers it. */
export const PERMANENT_DELETE_COPY =
  'Removes it for everyone. Earlier encrypted versions may remain on the server.';

/** What an entry in Trash is called: its original name where recorded. */
function trashLabel(item: Item): string {
  const parsed = parseTrashName(nameOf(item.path));
  return parsed?.kind === 'path' ? nameOf(parsed.original) : nameOf(item.path);
}

/** Why a team member is offered permanent deletion instead of Trash. */
export const TRASH_UNAVAILABLE_COPY =
  'Trash is not set up in this team yet, and only an owner can set it up by moving any item to Trash.';

/**
 * Confirmation dialog for permanently deleting an entry in Trash, or an item
 * in a team whose Trash this account cannot set up.
 */
function DeleteSheet({
  workflow,
  setWorkflow,
  onDeleteConflict,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'delete' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onDeleteConflict: (item: Item) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    accessNow,
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
  } = useShellServices();
  const [deleting, setDeleting] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [failureMessage, setFailureMessage] = useState<string | null>(null);
  const [, recheckAccess] = useState(0);
  const accessDescriptionId = useId();
  const problem = itemActionProblem(snapshot, workflow.item, true, accessNow());
  useSheetGuard(
    deleting
      ? { verdict: 'refuse', reason: 'Wait for the deletion to finish.' }
      : null,
    !deleting,
  );
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      dismissible={!deleting}
      title={`Delete ${trashLabel(workflow.item)} permanently?`}
      footer={
        <>
          <Button disabled={deleting} onClick={() => setWorkflow(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={deleting || uncertain || Boolean(problem)}
            title={problem}
            aria-describedby={problem ? accessDescriptionId : undefined}
            onClick={() => {
              if (deleting || uncertain) return;
              if (
                itemActionProblem(snapshot, workflow.item, true, accessNow())
              ) {
                recheckAccess((revision) => revision + 1);
                return;
              }
              setDeleting(true);
              setFailureMessage(null);
              void (async () => {
                try {
                  await bridge.removeItem({
                    direntId: workflow.item.direntId,
                    storeId: workflow.item.store,
                    path: workflow.item.path,
                    version: workflow.item.version,
                    ...(workflow.outsideTrash ? { outsideTrash: true } : {}),
                  });
                  setWorkflow(null);
                  // Keep refresh failure separate from the acknowledged deletion.
                  try {
                    await onApplied(
                      `Deleted ${trashLabel(workflow.item)} permanently`,
                      storeOf(snapshot, workflow.item.store)?.server,
                    );
                  } catch (failure) {
                    await onMutationError(failure, { report: false });
                  }
                } catch (error) {
                  const typed = normalizeCommandError(error);
                  setUncertain(typed.ambiguous);
                  setFailureMessage(typed.message);
                  if (typed.code === 'conflict') {
                    // Changed or already gone: the shell's refresh finds out
                    // which and says so, rather than this sheet assuming.
                    setWorkflow(null);
                    await onDeleteConflict(workflow.item);
                  } else {
                    await onMutationError(error, { item: workflow.item });
                  }
                } finally {
                  setDeleting(false);
                }
              })();
            }}
          >
            {deleting ? 'Deleting…' : 'Delete permanently'}
          </Button>
        </>
      }
    >
      {failureMessage ? <p role="alert">{failureMessage}</p> : null}
      {problem ? (
        <p id={accessDescriptionId} className="action-error" role="status">
          {problem}
        </p>
      ) : null}
      {workflow.outsideTrash ? <p>{TRASH_UNAVAILABLE_COPY}</p> : null}
      <p>
        {workflow.item.kind === 'Folder'
          ? `This folder and everything in it will be deleted. ${PERMANENT_DELETE_COPY}`
          : PERMANENT_DELETE_COPY}
      </p>
    </SheetDialog>
  );
}

/**
 * Restoring an entry from Trash found another item at its original path.
 * The entry can be restored under a new name beside it; nothing is replaced.
 */
function RestoreConflictSheet({
  workflow,
  setWorkflow,
  onRestoreTrash,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'restore-conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRestoreTrash: (item: Item, destination: string) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const [restoring, setRestoring] = useState(false);
  const { item, destination } = workflow;
  const copy = restoredCopyPath(destination, (path) =>
    snapshot.items.some(
      (candidate) => candidate.store === item.store && candidate.path === path,
    ),
  );
  const occupant = snapshot.items.find(
    (candidate) =>
      candidate.store === item.store && candidate.path === destination,
  );
  useSheetGuard(
    restoring
      ? { verdict: 'refuse', reason: 'Wait for the restore to finish.' }
      : null,
    !restoring,
  );
  return (
    <SheetDialog
      onClose={() => setWorkflow(null)}
      dismissible={!restoring}
      title={`${nameOf(destination)} is already in use`}
      footer={
        <>
          <Button
            disabled={restoring}
            data-dialog-autofocus="true"
            onClick={() => setWorkflow(null)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={restoring}
            onClick={() => {
              if (restoring) return;
              setRestoring(true);
              void onRestoreTrash(item, copy).finally(() => {
                setRestoring(false);
                setWorkflow(null);
              });
            }}
          >
            {restoring ? 'Restoring…' : `Restore as ${nameOf(copy)}`}
          </Button>
        </>
      }
    >
      <p>
        Another {occupant?.kind === 'Folder' ? 'folder' : 'item'} is at{' '}
        <code>{destination}</code>. The entry from Trash can be restored beside
        it under a new name; neither is replaced.
      </p>
      <Inset>
        <InsetRow label="In use">
          <code>{destination}</code>
        </InsetRow>
        <InsetRow label="Restores to">
          <code>{copy}</code>
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}

/**
 * Permanently deletes every entry in one store's Trash that this account can
 * change, folders with everything in them, in one host mutation. Entries
 * whose write role this account lacks stay in Trash.
 */
function EmptyTrashSheet({
  workflow,
  setWorkflow,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'empty-trash' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    accessNow,
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
  } = useShellServices();
  const [listed] = useState(() => trashEntries(snapshot, workflow.store));
  const entries = useTrashOrigins(bridge, listed);
  const [emptying, setEmptying] = useState(false);
  const store = storeOf(snapshot, workflow.store);
  const problems = entries.map((entry) =>
    itemActionProblem(snapshot, entry.item, true, accessNow()),
  );
  const removable = entries.filter((_, index) => !problems[index]);
  const kept = entries.length - removable.length;
  const problem = removable.length ? undefined : problems[0];
  useSheetGuard(
    emptying
      ? { verdict: 'refuse', reason: 'Wait for Trash to finish emptying.' }
      : null,
    !emptying,
  );
  const count = (amount: number): string =>
    `${amount} ${amount === 1 ? 'item' : 'items'}`;
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      dismissible={!emptying}
      title={`Permanently delete ${count(removable.length)}?`}
      footer={
        <>
          <Button
            disabled={emptying}
            data-dialog-autofocus="true"
            onClick={() => setWorkflow(null)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={emptying || !removable.length}
            title={problem}
            onClick={() => {
              if (emptying || !removable.length) return;
              setEmptying(true);
              void (async () => {
                try {
                  await bridge.emptyTrash({
                    storeId: workflow.store,
                    entries: removable.map(({ item }) => ({
                      path: item.path,
                      version: item.version,
                      direntId: item.direntId,
                    })),
                  });
                } catch (error) {
                  setWorkflow(null);
                  await onMutationError(error);
                  return;
                } finally {
                  setEmptying(false);
                }
                setWorkflow(null);
                try {
                  await onApplied(
                    `Emptied Trash in ${store?.name ?? 'this vault'}`,
                    store?.server,
                  );
                } catch (failure) {
                  await onMutationError(failure, { report: false });
                }
              })();
            }}
          >
            {emptying ? 'Deleting…' : 'Empty Trash'}
          </Button>
        </>
      }
    >
      {problem ? (
        <p className="action-error" role="status">
          {problem}
        </p>
      ) : null}
      <p>{PERMANENT_DELETE_COPY}</p>
      {kept && removable.length ? (
        <p>
          {count(kept)} {kept === 1 ? 'stays' : 'stay'} in Trash because this
          account cannot change {kept === 1 ? 'it' : 'them'}.
        </p>
      ) : null}
      <ul className="trash-empty-list">
        {entries.map((entry, index) => (
          <li
            key={entry.item.path}
            className={problems[index] ? 'kept' : undefined}
          >
            <Icon name={entry.item.kind === 'Folder' ? 'folder' : 'file'} />
            <span>{trashEntryLabel(entry)}</span>
            {entry.item.kind === 'Folder' ? (
              <small>
                folder · {entry.count} {entry.count === 1 ? 'item' : 'items'}
              </small>
            ) : null}
            {problems[index] ? <small>stays in Trash</small> : null}
          </li>
        ))}
      </ul>
    </SheetDialog>
  );
}
