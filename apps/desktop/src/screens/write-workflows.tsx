import { useEffect, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Dialog, DismissibleDialog } from '/kit/overlay-primitives';
import {
  Band,
  Button,
  CardSelect,
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
import type { CardOption, FilterKind } from '../components';
import {
  isLogin,
  itemKey,
  kindLabel,
  kindOf,
  canCreateInStore,
  canChangeItem,
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
  storeReadable,
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
import type { MutationFailureHandler } from '../mutation-recovery';
import { useToast } from '/kit/toasts';
import { editableValue } from './edit-value';

export type NewKind = Exclude<ItemKind, 'Folder'>;

interface NewDraft {
  path: string;
  site: string;
  username: string;
  password: string;
  website: string;
  value: string;
  resourceName: string;
  target: string;
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
  | { kind: 'agent-lost'; message?: string }
  | null;

const DRAFT_PATH: Readonly<Record<NewKind, string>> = {
  Password: '/logins/github.com',
  Resource: '/agents/anthropic-api-key',
  File: '/documents/emergency.pdf',
  Link: '/latest-key',
};

function defaultPath(itemKind: NewKind, folder?: string): string {
  if (!folder || folder === '/') return DRAFT_PATH[itemKind];
  const name = DRAFT_PATH[itemKind].split('/').at(-1) ?? 'new-item';
  return `${folder.replace(/\/$/, '')}/${name}`;
}

function fileDropPath(name: string, folder?: string): string {
  return folder && folder !== '/'
    ? `${folder.replace(/\/$/, '')}/${name}`
    : `/documents/${name}`;
}

function namedPath(
  name: string,
  defaultDirectory: string,
  folder?: string,
): string {
  const directory =
    folder && folder !== '/' ? folder.replace(/\/$/, '') : defaultDirectory;
  return `${directory}/${name}`;
}

export function initialWriteWorkflow(
  search: string,
  snapshot?: AgentSnapshot,
): WriteWorkflow {
  const state = new URLSearchParams(search).get('state');
  if (state === 'new')
    return { kind: 'new', itemKind: 'Password', storeId: 'team:household' };
  if (state === 'new-group')
    return { kind: 'new', itemKind: 'Resource', storeId: 'team:eng' };
  if (state === 'new-resource')
    return { kind: 'new', itemKind: 'Resource', storeId: 'acct:personal' };
  if (state === 'new-file')
    return { kind: 'new', itemKind: 'File', storeId: 'team:household' };
  if (state === 'new-link')
    return { kind: 'new', itemKind: 'Link', storeId: 'acct:personal' };
  if (state === 'group-new-text')
    return { kind: 'new', itemKind: 'Resource', storeId: 'team:eng' };
  if (state === 'group-new-link')
    return { kind: 'new', itemKind: 'Link', storeId: 'team:eng' };
  if (state === 'group-new-file')
    return { kind: 'new', itemKind: 'File', storeId: 'team:eng' };
  if (state === 'exists') {
    return {
      kind: 'exists',
      itemKind: 'Password',
      storeId: 'acct:personal',
      path: DRAFT_PATH.Password,
    };
  }
  if (state === 'agent-lost') return { kind: 'agent-lost' };
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
  if (typed.code === 'agent-lost')
    return { kind: 'agent-lost', message: typed.message };
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
 * Formats a store option for the "Save in" vault selector.
 */
function storeOption(snapshot: AgentSnapshot, store: Store): CardOption {
  return {
    id: store.id,
    title: store.name,
    detail: storeDescription(snapshot, store),
    off: !canCreateInStore(snapshot, store.id),
  };
}

/**
 * Returns a user-facing explanation if the selected store is read-only or blocked.
 */
function writeBlockReason(
  snapshot: AgentSnapshot,
  store: Store,
): string | null {
  if (canCreateInStore(snapshot, store.id)) return null;
  if (leaseLapsed(snapshot, store.id))
    return 'The signed server check-in expired. Check in again before retrying.';
  if (serverBlocked(snapshot, store.id))
    return 'Server access is blocked. Cannot create items in this vault.';
  if (serverLeaseUnavailable(snapshot, store.id))
    return 'No usable signed server check-in is available. Cannot create items in this vault.';
  if (store.kind === 'team' && !store.active)
    return 'Group setup is incomplete. Complete setup before adding items.';
  if (store.kind === 'team')
    return 'You do not have write permissions for this group.';
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
  const [storeId, setStoreId] = useState(() =>
    storeOf(snapshot, workflow.storeId)
      ? workflow.storeId
      : (defaultCreateStore(snapshot) ?? workflow.storeId),
  );
  const [site, setSite] = useState(workflow.draft?.site ?? '');
  const [path, setPath] = useState(
    workflow.draft?.path ??
      (workflow.itemKind === 'Password'
        ? namedPath(
            workflow.draft?.site ?? '',
            '/logins',
            workflow.initialFolder,
          )
        : defaultPath(workflow.itemKind, workflow.initialFolder)),
  );
  const [username, setUsername] = useState(workflow.draft?.username ?? '');
  const [password, setPassword] = useState(workflow.draft?.password ?? '');
  const [website, setWebsite] = useState(workflow.draft?.website ?? '');
  const [value, setValue] = useState(workflow.draft?.value ?? '');
  const [resourceName, setResourceName] = useState(
    workflow.draft?.resourceName ?? '',
  );
  const [target, setTarget] = useState(workflow.draft?.target ?? '');
  const [sourcePath, setSourcePath] = useState<string | null>(
    workflow.draft?.sourcePath ?? null,
  );
  const [readRole, setReadRole] = useState<KvRoleInput>(
    workflow.draft?.readRole ?? 'Member:0',
  );
  const [writeRole, setWriteRole] = useState<KvRoleInput>(
    workflow.draft?.writeRole ?? 'Admin',
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

  useEffect(() => {
    if (itemKind !== 'File') return;
    let disposed = false;
    let hoverOff: (() => void) | undefined;
    let pathsOff: (() => void) | undefined;
    void bridge
      .onDropHover(({ hovering: next }) => setHovering(next))
      .then(
        (off) => {
          if (disposed) off();
          else hoverOff = off;
        },
        (error) => {
          if (!disposed) setFileError(normalizeCommandError(error).message);
        },
      );
    void bridge
      .onDropPaths((paths) => {
        if (paths.length !== 1) {
          setSourcePath(null);
          setHovering(false);
          setFileError('Drop exactly one file.');
          return;
        }
        const first = paths[0];
        if (!first) return;
        setSourcePath(first);
        setFileError(null);
        const name = first.split(/[\\/]/).at(-1);
        if (name) setPath(fileDropPath(name, workflow.initialFolder));
        setHovering(false);
      })
      .then(
        (off) => {
          if (disposed) off();
          else pathsOff = off;
        },
        (error) => {
          if (!disposed) setFileError(normalizeCommandError(error).message);
        },
      );
    return () => {
      disposed = true;
      hoverOff?.();
      pathsOff?.();
    };
  }, [bridge, itemKind, workflow.initialFolder]);

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
      } else if (itemKind === 'Resource') {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value,
          ...roleArgs,
        });
      } else if (itemKind === 'Link') {
        await bridge.createLink({
          storeId: store.id,
          path,
          target,
          ...roleArgs,
        });
      } else if (sourcePath) {
        await bridge.importDroppedFile({
          storeId: store.id,
          path,
          sourcePath,
          ...roleArgs,
        });
      } else {
        const result = await bridge.pickAndImportFile({
          storeId: store.id,
          path,
          ...roleArgs,
        });
        if (!result.applied) return;
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
            target,
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
  return (
    <Sheet
      width="mid"
      glyph={<KindIcon kind={itemKind} />}
      title={`New ${kindLabel(itemKind).toLowerCase()}`}
      subtitle={
        itemKind === 'Password'
          ? 'A saved login with username and password credentials.'
          : itemKind === 'Resource'
            ? 'A value such as an API key or recovery code.'
            : itemKind === 'File'
              ? 'A file stored securely in this vault.'
              : 'A reference pointing to another item in the same vault.'
      }
      footer={
        <>
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!canWrite || saving || !pathName}
            onClick={() => void submit()}
          >
            {saving
              ? 'Creating…'
              : itemKind === 'File' && !sourcePath
                ? 'Choose file and create'
                : 'Create item'}
          </Button>
        </>
      }
    >
      <>
        <SectionLabel>Save in</SectionLabel>
        <Inset>
          {snapshot.stores.length ? (
            <CardSelect
              label="Save in"
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
        {store && blocked ? <Band>{`${store.name}: ${blocked}`}</Band> : null}
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
        <SectionLabel>{kindLabel(itemKind)}</SectionLabel>
        <Inset
          className={itemKind === 'File' && hovering ? 'drop-hover' : undefined}
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
          {itemKind === 'Resource' ? (
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
                        '/agents',
                        workflow.initialFolder,
                      ),
                    );
                },
                'e.g. ANTHROPIC_API_KEY',
              )}
              {field('Value', value, setValue, 'sk-ant-…', true)}
            </>
          ) : null}
          {itemKind === 'Link'
            ? field('Target path', target, setTarget, '/ssh/id_ed25519', true)
            : null}
          {itemKind === 'File' ? (
            <InsetRow label="File">
              <span className={sourcePath ? 'mono' : 'dim'}>
                {sourcePath
                  ? sourcePath.split(/[\\/]/).at(-1)
                  : hovering
                    ? 'Drop to use this file'
                    : 'Drop a file here, or choose a file'}
              </span>
            </InsetRow>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
        <Toggle label="Advanced" className="sheet-advanced">
          <Inset>{field('Path', path, setPath, DRAFT_PATH[itemKind])}</Inset>
          <p className="hint">The item will be saved at this vault path.</p>
        </Toggle>
      </>
    </Sheet>
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
  const version = clash?.version;
  return (
    <Sheet
      glyph={
        clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : undefined
      }
      title={`An item already exists at ${workflow.path}`}
      subtitle={`New ${kindLabel(workflow.itemKind).toLowerCase()} · not saved`}
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
          An item already exists at this path. No files were overwritten
          {version
            ? ` in ${storeOf(snapshot, workflow.storeId)?.name ?? 'this vault'} (currently version ${version})`
            : ''}
          . You can open the existing item to review it, or choose a different
          name or path.
        </p>
      </>
    </Sheet>
  );
}

function AgentLostDialog({
  message,
  bridge,
  onRetryAgent,
  onReconnected,
}: {
  message?: string;
  bridge: Bridge;
  onRetryAgent: () => Promise<void>;
  onReconnected: () => void;
}): ReactNode {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  return (
    <Dialog
      className="stopwrap"
      role="alertdialog"
      aria-label="Connection to background service lost"
    >
      <div className="notice stop">
        <h2>Connection to background service lost</h2>
        <p>
          The background service stopped responding. Click Retry to reconnect.
        </p>
        {message ? <p className="fn">{message}</p> : null}
        {failure ? (
          <p className="fn" role="alert">
            {failure}
          </p>
        ) : null}
        <div className="acts2">
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              setFailure(null);
              void (async () => {
                try {
                  await onRetryAgent();
                  onReconnected();
                } catch (error) {
                  setFailure(normalizeCommandError(error).message);
                } finally {
                  setBusy(false);
                }
              })();
            }}
          >
            Retry
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              void bridge
                .quitApp()
                .catch((error) =>
                  setFailure(normalizeCommandError(error).message),
                );
            }}
          >
            Quit FOKS
          </Button>
        </div>
      </div>
    </Dialog>
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
  onRetryAgent,
  onRefreshConflict,
  onDiscardConflict,
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
  onRetryAgent: () => Promise<void>;
  onRefreshConflict: (item: Item, draft: string) => Promise<void>;
  onDiscardConflict: () => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  accessNow?: () => number;
}): ReactNode {
  if (!workflow) return null;
  if (workflow.kind === 'agent-lost')
    return (
      <AgentLostDialog
        message={workflow.message}
        bridge={bridge}
        onRetryAgent={onRetryAgent}
        onReconnected={() => setWorkflow(null)}
      />
    );
  if (workflow.kind === 'new')
    return (
      <DismissibleDialog
        className="backdrop"
        aria-label={`New ${kindLabel(workflow.itemKind).toLowerCase()}`}
        onDismiss={() => setWorkflow(null)}
      >
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
      </DismissibleDialog>
    );
  if (workflow.kind === 'exists')
    return (
      <DismissibleDialog
        className="backdrop"
        aria-label="Item path already exists"
        onDismiss={() => setWorkflow(null)}
      >
        <ExistsSheet
          snapshot={snapshot}
          workflow={workflow}
          setWorkflow={setWorkflow}
          onOpenExisting={onOpenExisting}
          onError={(error) => onError(error)}
        />
      </DismissibleDialog>
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
  onRefreshConflict: (item: Item, draft: string) => Promise<void>;
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
          subtitle={`${nameOf(workflow.item.path)} · Unsaved changes`}
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
          <p>
            Your draft has not been saved. Discarding will permanently delete
            your changes. The existing item will remain unchanged.
          </p>
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
        title="Item modified elsewhere"
        subtitle="A newer version was saved from another device. Choose which version to keep."
        footer={
          <>
            <Button onClick={() => setConfirmingDiscard(true)}>
              Discard my edit
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                void onRefreshConflict(workflow.item, workflow.draft).then(
                  () => setWorkflow(null),
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
            A newer version was saved while you were editing. Review the latest
            version and reapply your changes.
          </p>
          <p className="fn">
            Refresh to compare the latest version with your draft, review
            changes, and save your update.
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
}: {
  snapshot: AgentSnapshot;
  accessNow: () => number;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'delete' }>;
  bridge: Bridge;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const toasts = useToast();
  const [deleting, setDeleting] = useState(false);
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
      subtitle={workflow.item.path}
      footer={
        <>
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            variant="primary"
            danger
            disabled={deleting}
            onClick={() => {
              const store = storeOf(snapshot, workflow.item.store);
              if (
                deleting ||
                !store ||
                !storeAvailability(snapshot, store, {
                  nowSeconds: accessNow(),
                }).available ||
                !storeReadable(snapshot, workflow.item.store) ||
                !canChangeItem(snapshot, workflow.item)
              )
                return;
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
                    setWorkflow(null);
                    await onMutationError(error, { report: false });
                    toasts.show(
                      'This item was modified by another user or session. Review the updated item before deleting.',
                      { tone: 'warning' },
                    );
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
      <p>
        This item will be permanently deleted. This action cannot be undone.
      </p>
    </SheetDialog>
  );
}
