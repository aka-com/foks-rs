import { useTabSheetState } from '../navigation-guard';
import { useCallback, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Dialog, DismissibleDialog } from '/kit/overlay-primitives';
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
  RadioCard,
  RadioGroup,
  SectionLabel,
  Sheet,
  SheetDialog,
  Toggle,
} from '../components';
import type { CardOption, DocumentSourceKind, FilterKind } from '../components';
import {
  isLogin,
  itemKey,
  kindLabel,
  kindOf,
  canCreateInStore,
  defaultCreateStore,
  leaseLapsed,
  nameOf,
  partiesOf,
  partyName,
  readersOf,
  serverBlocked,
  serverLeaseUnavailable,
  storeDescription,
  storeNavigationOrder,
  storeAvailability,
  storeOf,
} from '../model';
import type {
  Item,
  ItemKind,
  RoleWire,
  Store,
  StoreRef,
  AgentSnapshot,
} from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge, KvRoleInput } from '../bridge';
import type { GuardVerdict } from '../location';
import { NavigationPrompt, useSheetGuard } from '../navigation-guard';
import { useFileDrop } from '../file-drop';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useToast } from '/kit/toasts';
import { editableValue } from './edit-value';
import { itemActionProblem } from './store-access';

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

/** Group items default to team-readable content that only admins can change. */
export const DEFAULT_READ_ROLE: KvRoleInput = 'Member:0';
export const DEFAULT_WRITE_ROLE: KvRoleInput = 'Admin';

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
      kind: 'exists';
      itemKind: NewKind;
      storeId: StoreRef;
      path: string;
      draft?: NewDraft;
    }
  | { kind: 'delete'; item: Item }
  | { kind: 'conflict'; item: Item; draft: string }
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
 * The new-item sheet an edit resumes in when the item it was editing has been
 * deleted elsewhere: there is no newer version to review, so the edit is
 * offered as a new item at the same path, for the user to save or discard.
 */
export function conflictDraftWorkflow(
  item: Item,
  draft: string,
): WriteWorkflow {
  const itemKind: NewKind =
    kindOf(item) === 'Password' ? 'Password' : 'Document';
  const fields = new Map<string, string>();
  if (itemKind === 'Password')
    for (const line of draft.split('\n')) {
      const at = line.indexOf(':');
      if (at > 0)
        fields.set(line.slice(0, at).trim(), line.slice(at + 1).trim());
    }
  return {
    kind: 'new',
    itemKind,
    storeId: item.store,
    draft: {
      path: item.path,
      site: itemKind === 'Password' ? nameOf(item.path) : '',
      username: fields.get('username') ?? fields.get('user') ?? '',
      password: fields.get('password') ?? '',
      website: fields.get('url') ?? fields.get('website') ?? '',
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
    };
  }
  if (state === 'conflict' && snapshot) {
    const item = snapshot.items.find((candidate) => isLogin(candidate));
    if (item)
      return {
        kind: 'conflict',
        item,
        draft: editableValue(
          item,
          snapshot.plaintext[itemKey(item)] ?? item.value ?? '',
        ),
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
  snapshot: AgentSnapshot;
  bridge: Bridge;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  accessNow: () => number;
}

/**
 * Formats a store option for the "Save in vault" selector.
 */
function storeOption(snapshot: AgentSnapshot, store: Store): CardOption {
  return {
    id: store.id,
    title: store.name,
    detail: `${store.kind === 'account' ? 'Personal vault' : 'Team vault'} · ${storeDescription(snapshot, store)}`,
    off: !canCreateInStore(snapshot, store.id),
  };
}

/**
 * Returns a user-facing explanation if the selected store is read-only or blocked.
 */
export function writeBlockReason(
  snapshot: AgentSnapshot,
  store: Store,
): string | null {
  if (canCreateInStore(snapshot, store.id)) return null;
  if (leaseLapsed(snapshot, store.id))
    return 'Your server session has expired. Check in again to save changes.';
  if (serverBlocked(snapshot, store.id))
    return 'Server access is blocked. Cannot create items in this vault.';
  if (serverLeaseUnavailable(snapshot, store.id))
    return 'The server connection is unavailable. New items cannot be saved to this vault right now.';
  if (store.kind === 'team' && !store.active)
    return 'Team setup is incomplete. Complete setup before adding items.';
  if (store.kind === 'team')
    return 'You do not have write permissions for this team.';
  return 'You do not have permission to create items in this vault.';
}

function AccessBlock({
  snapshot,
  store,
  readRole,
  writeRole,
  onReadRole,
  onWriteRole,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  readRole: KvRoleInput;
  writeRole: KvRoleInput;
  onReadRole: (role: KvRoleInput) => void;
  onWriteRole: (role: KvRoleInput) => void;
}): ReactNode {
  if (store.kind !== 'team') {
    return null;
  }
  const roleWire = (role: KvRoleInput): RoleWire =>
    role === 'Owner' || role === 'Admin'
      ? { role }
      : { role: 'Member', visibility: Number(role.slice('Member:'.length)) };
  const candidate: Item = {
    store: store.id,
    path: '/preview',
    kind: 'Secret',
    size: 0,
    version: 0,
    read: roleWire(readRole),
    write: roleWire(writeRole),
  };
  const roster = partiesOf(snapshot, store.id);
  const admitted = readersOf(snapshot, candidate) ?? [];
  const excluded = roster.filter((party) => !admitted.includes(party));
  const changers =
    readersOf(snapshot, { ...candidate, read: candidate.write }) ?? [];
  const readVisibility = readRole.startsWith('Member:')
    ? Number(readRole.slice('Member:'.length))
    : 0;
  const choices: readonly [string, KvRoleInput, string][] = [
    ['Owner', 'Owner', 'Only owners can read.'],
    ['Admin', 'Admin', 'Admins and owners can read.'],
    [
      'Member',
      `Member:${readVisibility}`,
      'Members at this visibility level and above can read.',
    ],
  ];
  const roleChoices = (
    selected: KvRoleInput,
    choose: (role: KvRoleInput) => void,
    forWrite: boolean,
  ) =>
    choices.map(([label, role, detail]) => {
      const next =
        label === 'Member'
          ? (`Member:${selected.startsWith('Member:') ? Number(selected.slice('Member:'.length)) : 0}` as KvRoleInput)
          : role;
      const on =
        selected === next ||
        (label === 'Member' && selected.startsWith('Member:'));
      return (
        <RadioCard
          key={label}
          selected={on}
          onSelect={() => choose(next)}
          title={label}
          detail={
            forWrite
              ? 'Members with Admin or Owner roles can modify or delete this item.'
              : detail
          }
        />
      );
    });
  return (
    <>
      <SectionLabel
        action={
          <span className="pv">
            accessible to{' '}
            <b>
              {admitted.length} of {roster.length}
            </b>
          </span>
        }
      >
        Who can read
      </SectionLabel>
      <Inset>
        <RadioGroup label="Who can read">
          {roleChoices(readRole, onReadRole, false)}
        </RadioGroup>
        {readRole.startsWith('Member:') ? (
          <Field
            label="Member visibility"
            type="number"
            mono
            min={-32768}
            max={32767}
            value={String(readVisibility)}
            onChange={(val) => {
              const next = Number(val);
              if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                onReadRole(`Member:${next}`);
            }}
          />
        ) : null}
      </Inset>
      <p className="hint">
        At{' '}
        <b>
          {readRole === 'Owner' || readRole === 'Admin'
            ? readRole
            : `Member · visibility ${readVisibility}`}
        </b>{' '}
        this item will be readable by{' '}
        <b>
          {admitted.length} of {roster.length}
        </b>{' '}
        members in {store.name}: {admitted.map(partyName).join(', ') || 'none'}.
        {excluded.length
          ? ` ${excluded.map(partyName).join(', ')} cannot read this item under the selected role.`
          : ''}
      </p>
      <SectionLabel>
        Who can change{' '}
        <span className="pv">
          changeable by{' '}
          <b>
            {changers.length} of {roster.length}
          </b>
        </span>
      </SectionLabel>
      <Inset>
        <RadioGroup label="Who can change">
          {roleChoices(writeRole, onWriteRole, true)}
        </RadioGroup>
        {writeRole.startsWith('Member:') ? (
          <Field
            label="Member visibility"
            type="number"
            mono
            min={-32768}
            max={32767}
            value={writeRole.slice('Member:'.length)}
            onChange={(val) => {
              const next = Number(val);
              if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                onWriteRole(`Member:${next}`);
            }}
          />
        ) : null}
      </Inset>
      <p className="hint">
        Read and write permissions are set separately. A member with write
        permission can update this item even if they cannot view its contents.
      </p>
    </>
  );
}

function NewSheet({
  snapshot,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onError,
  onMutationError,
  accessNow,
}: NewSheetProps): ReactNode {
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
  const [username, setUsername] = useTabSheetState(
    'item.username',
    workflow.draft?.username ?? '',
  );
  const [password, setPassword] = useTabSheetState(
    'item.password',
    workflow.draft?.password ?? '',
  );
  const [website, setWebsite] = useTabSheetState(
    'item.website',
    workflow.draft?.website ?? '',
  );
  const [value, setValue] = useTabSheetState(
    'item.value',
    workflow.draft?.value ?? '',
  );
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
  const roleArgs = group ? { readRole, writeRole } : {};

  useFileDrop({
    bridge,
    active: itemKind === 'Document',
    onHover: setHovering,
    onPaths: (paths) => {
      setHovering(false);
      if (paths.length !== 1) {
        setSourcePath(null);
        setFileError('Drop exactly one file.');
        return;
      }
      const first = paths[0];
      if (!first) return;
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
  const close = useCallback(() => setWorkflow(null), [setWorkflow]);
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
    try {
      if (itemKind === 'Password') {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value: `user: ${username}\npassword: ${password}\nurl: ${website}`,
          ...roleArgs,
        });
      } else if (source === 'file' && sourcePath) {
        await bridge.importDroppedFile({
          storeId: store.id,
          path,
          sourcePath,
          ...roleArgs,
        });
      } else if (source === 'file') {
        const result = await bridge.pickAndImportFile({
          storeId: store.id,
          path,
          ...roleArgs,
        });
        if (!result.applied) return;
      } else {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value,
          ...roleArgs,
        });
      }
      await onApplied(`${kindLabel(itemKind)} created in ${store.name}`);
      setWorkflow(null);
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'already-exists') {
        setWorkflow({
          kind: 'exists',
          itemKind,
          storeId: store.id,
          path,
          draft: {
            path,
            site,
            username,
            password,
            website,
            value,
            resourceName,
            sourcePath,
            readRole,
            writeRole,
          },
        });
        await onMutationError(error, { report: false });
      } else if (typed.code === 'inactive-group') {
        try {
          await onApplied(`Cannot create item: ${store.name} is inactive`);
          setWorkflow(null);
        } catch (refreshError) {
          onError(refreshError);
        }
      } else await onMutationError(error);
    } finally {
      setSaving(false);
    }
  };

  const field = (
    label: string,
    current: string,
    set: (next: string) => void,
    placeholder: string,
    mono = false,
    type: 'text' | 'password' = 'text',
  ) => (
    <Field
      label={label}
      value={current}
      onChange={set}
      type={type}
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
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
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
        <SectionLabel>Save in vault</SectionLabel>
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
        </Inset>
        {store && blocked ? (
          <Band live>{`${store.name}: ${blocked}`}</Band>
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
              {field('Password', password, setPassword, '', false, 'password')}
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
              />
            </>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
        <Toggle label="Advanced" className="sheet-advanced">
          <Inset>{field('Path', path, setPath, PATH_HINT[itemKind])}</Inset>
        </Toggle>
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

function ExistsSheet({
  snapshot,
  workflow,
  setWorkflow,
  onOpenExisting,
  onError,
}: {
  snapshot: AgentSnapshot;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const clash = snapshot.items.find(
    (item) => item.store === workflow.storeId && item.path === workflow.path,
  );
  // This step still holds the draft the new-item sheet carried here, so it is
  // abandoned under the same confirmation, from a navigation or from Escape
  // and the backdrop.
  const close = useCallback(() => setWorkflow(null), [setWorkflow]);
  useSheetGuard(discardNewItem(workflow.itemKind, close));
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const sheet = (
    <Sheet
      glyph={
        clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : undefined
      }
      title={`An item already exists at ${workflow.path}`}
      footer={
        <>
          <Button
            onClick={() =>
              setWorkflow({
                kind: 'new',
                itemKind: workflow.itemKind,
                storeId: workflow.storeId,
                draft: workflow.draft,
              })
            }
          >
            Change path
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              void onOpenExisting(workflow).then(
                () => setWorkflow(null),
                onError,
              );
            }}
          >
            Open existing item
          </Button>
        </>
      }
    >
      <>
        <p>
          Nothing was overwritten. Open the existing item, or choose another
          path.
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
  snapshot,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onError,
  onMutationError,
  onRefreshConflict,
  onDiscardConflict,
  onDeleteConflict,
  onOpenExisting,
  accessNow = () => Date.now() / 1000,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  workflow: WriteWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown, item?: Item) => void;
  onMutationError: MutationFailureHandler;
  /**
   * Refreshes after an edit conflict and says what the conflict sheet gives
   * way to: nothing, when the item is there to review, or a new-item sheet
   * carrying the edit when the item turned out to have been deleted.
   */
  onRefreshConflict: (item: Item, draft: string) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
  /** Refreshes after a delete conflict and states whether the item is still there. */
  onDeleteConflict: (item: Item) => Promise<void>;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  accessNow?: () => number;
}): ReactNode {
  if (!workflow) return null;
  // The sheet supplies its own dialog: Escape and the backdrop are decided by
  // the draft it holds, the same state its navigation guard answers from.
  if (workflow.kind === 'new')
    return (
      <NewSheet
        snapshot={snapshot}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={onApplied}
        onError={(error) => onError(error)}
        onMutationError={onMutationError}
        accessNow={accessNow}
      />
    );
  // The already-exists step carries the same draft, so it supplies its own
  // dialog for the same reason.
  if (workflow.kind === 'exists')
    return (
      <ExistsSheet
        snapshot={snapshot}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onOpenExisting={onOpenExisting}
        onError={(error) => onError(error)}
      />
    );
  if (workflow.kind === 'conflict')
    return (
      <ConflictSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onRefreshConflict={onRefreshConflict}
        onDiscardConflict={onDiscardConflict}
        onError={onError}
      />
    );
  return (
    <DeleteSheet
      snapshot={snapshot}
      accessNow={accessNow}
      workflow={workflow}
      bridge={bridge}
      setWorkflow={setWorkflow}
      onApplied={onApplied}
      onMutationError={onMutationError}
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
  onError,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRefreshConflict: (item: Item, draft: string) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
  onError: (error: unknown, item?: Item) => void;
}): ReactNode {
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);

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
                Keep editing
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
      aria-label="Edit conflict"
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
              Discard my edit
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                // The refresh finds out whether the item was changed or
                // removed, and says what this sheet gives way to.
                void onRefreshConflict(workflow.item, workflow.draft).then(
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
            This item was changed or removed while you were editing. Refresh to
            see what happened; your edit is kept until you discard it.
          </p>
        </>
      </Sheet>
    </Dialog>
  );
}

/**
 * Confirmation dialog for deleting an item.
 */
function DeleteSheet({
  snapshot,
  accessNow,
  workflow,
  bridge,
  setWorkflow,
  onApplied,
  onMutationError,
  onDeleteConflict,
}: {
  snapshot: AgentSnapshot;
  accessNow: () => number;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'delete' }>;
  bridge: Bridge;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
  onDeleteConflict: (item: Item) => Promise<void>;
}): ReactNode {
  const [deleting, setDeleting] = useState(false);
  const [, recheckAccess] = useState(0);
  const accessDescriptionId = useId();
  const problem = itemActionProblem(snapshot, workflow.item, true, accessNow());
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      title={`Delete ${nameOf(workflow.item.path)}?`}
      footer={
        <>
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            variant="primary"
            danger
            disabled={deleting || Boolean(problem)}
            title={problem}
            aria-describedby={problem ? accessDescriptionId : undefined}
            onClick={() => {
              if (deleting) return;
              if (
                itemActionProblem(snapshot, workflow.item, true, accessNow())
              ) {
                recheckAccess((revision) => revision + 1);
                return;
              }
              setDeleting(true);
              void (async () => {
                try {
                  await bridge.removeItem({
                    storeId: workflow.item.store,
                    path: workflow.item.path,
                    version: workflow.item.version,
                  });
                  await onApplied(`Deleted ${nameOf(workflow.item.path)}`);
                  setWorkflow(null);
                } catch (error) {
                  const typed = normalizeCommandError(error);
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
            {deleting ? 'Deleting…' : 'Delete'}
          </Button>
        </>
      }
    >
      {problem ? (
        <p id={accessDescriptionId} className="action-error" role="status">
          {problem}
        </p>
      ) : null}
      <p>This cannot be undone.</p>
    </SheetDialog>
  );
}
