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
} from '../components';
import type { CardOption, FilterKind } from '../components';
import {
  isLogin,
  itemKey,
  kindLabel,
  kindOf,
  canCreateInStore,
  leaseLapsed,
  nameOf,
  partiesOf,
  partyName,
  readersOf,
  serverBlocked,
  serverLeaseUnavailable,
  storeDescription,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type {
  Item,
  ItemKind,
  RoleWire,
  Store,
  StoreRef,
  World,
} from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge, KvRoleInput } from '../bridge';
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
  | { kind: 'new'; itemKind: NewKind; storeId: StoreRef; draft?: NewDraft }
  | {
      kind: 'exists';
      itemKind: NewKind;
      storeId: StoreRef;
      path: string;
      draft?: NewDraft;
    }
  | { kind: 'remove'; item: Item }
  | { kind: 'conflict'; item: Item; draft: string }
  | { kind: 'agent-lost'; message?: string }
  | null;

const DRAFT_PATH: Readonly<Record<NewKind, string>> = {
  Password: '/logins/github.com',
  Resource: '/agents/anthropic-api-key',
  File: '/documents/emergency.pdf',
  Link: '/latest-key',
};

export function initialWriteWorkflow(
  search: string,
  world?: World,
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
  if (state === 'conflict' && world) {
    const item = world.items.find((candidate) => isLogin(candidate));
    if (item)
      return {
        kind: 'conflict',
        item,
        draft: editableValue(
          item,
          world.plaintext[itemKey(item)] ?? item.value ?? '',
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
  world: World;
  bridge: Bridge;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
}

/**
 * One vault, group or ad-hoc share as the "Save in" dropdown draws it.
 *
 * The reading is `storeDescription` — the same sentence the sidebar row and
 * the page header give this store. The sheet used to compose its own, so a
 * group read "shared with 3 people · local" here and "3 people" two inches
 * to the left, and a lapsed server was explained in words no other surface
 * used. Whether the store can be written to is a separate question, and it
 * is the one `off` answers.
 */
function storeOption(world: World, store: Store): CardOption {
  return {
    id: store.id,
    title: store.name,
    detail: storeDescription(world, store),
    off: !canCreateInStore(world, store.id),
  };
}

/**
 * Why the chosen store cannot take a new item, when it cannot.
 *
 * The chooser reads the way the sidebar reads, and that sentence says what a
 * store *is*, never whether this sheet may write to it. The reason belongs
 * with the choice rather than inside its label, so it is drawn as a band
 * under the chooser — otherwise a disabled Create button would be the only
 * word on the subject.
 */
function writeBlockReason(world: World, store: Store): string | null {
  if (canCreateInStore(world, store.id)) return null;
  if (leaseLapsed(world, store.id))
    return 'server check-in lapsed — nothing can be written here';
  if (serverBlocked(world, store.id))
    return 'server access is blocked — nothing can be written here';
  if (serverLeaseUnavailable(world, store.id))
    return 'check-in status unknown — nothing can be written here';
  if (store.kind === 'team' && !store.active)
    return 'reports inactive — resume its creation first';
  if (store.kind === 'team')
    return 'no authenticated local group identity — writing is unavailable';
  return 'nothing can be written here';
}

function AccessBlock({
  world,
  store,
  readRole,
  writeRole,
  onReadRole,
  onWriteRole,
}: {
  world: World;
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
  const roster = partiesOf(world, store.id);
  const admitted = readersOf(world, candidate) ?? [];
  const excluded = roster.filter((party) => !admitted.includes(party));
  const changers =
    readersOf(world, { ...candidate, read: candidate.write }) ?? [];
  const readVisibility = readRole.startsWith('Member:')
    ? Number(readRole.slice('Member:'.length))
    : 0;
  const choices: readonly [string, KvRoleInput, string][] = [
    ['Owner', 'Owner', 'Only owners.'],
    ['Admin', 'Admin', 'Admins and owners.'],
    [
      'Member',
      `Member:${readVisibility}`,
      'Members at this visibility level and above.',
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
            forWrite ? detail.replace('.', ' can change or remove it.') : detail
          }
        />
      );
    });
  return (
    <>
      <SectionLabel
        action={
          <span className="pv">
            would be readable by{' '}
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
          <InsetRow label="Member visibility">
            <input
              type="number"
              min={-32768}
              max={32767}
              aria-label="Read visibility"
              value={readVisibility}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                  onReadRole(`Member:${next}`);
              }}
            />
          </InsetRow>
        ) : null}
      </Inset>
      <p className="hint">
        At{' '}
        <b>
          {readRole === 'Owner' || readRole === 'Admin'
            ? readRole
            : `Member · visibility ${readVisibility}`}
        </b>{' '}
        this item would be readable by{' '}
        <b>
          {admitted.length} of {roster.length}
        </b>{' '}
        in {store.name}: {admitted.map(partyName).join(', ') || 'nobody'}.
        {excluded.length
          ? ` ${excluded.map(partyName).join(', ')} is not counted when its role or group membership provides no access here.`
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
          <InsetRow label="Member visibility">
            <input
              type="number"
              min={-32768}
              max={32767}
              aria-label="Write visibility"
              value={Number(writeRole.slice('Member:'.length))}
              onChange={(event) => {
                const next = Number(event.target.value);
                if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                  onWriteRole(`Member:${next}`);
              }}
            />
          </InsetRow>
        ) : null}
      </Inset>
      <p className="hint">
        The read and write roles are independent and carried by this group item,
        so someone allowed to change it might not be allowed to read it. They
        are checked against the current authenticated roster, so the preview is
        computed rather than typed.
      </p>
    </>
  );
}

function NewSheet({
  world,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onError,
}: NewSheetProps): ReactNode {
  const [storeId, setStoreId] = useState(workflow.storeId);
  const [path, setPath] = useState(
    workflow.draft?.path ?? DRAFT_PATH[workflow.itemKind],
  );
  const [site, setSite] = useState(
    workflow.draft?.site ??
      (workflow.itemKind === 'Password' ? 'github.com' : ''),
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
  const store = storeOf(world, storeId);
  const group = store?.kind === 'team';
  const canWrite = Boolean(store && canCreateInStore(world, store.id));
  const blocked = store ? writeBlockReason(world, store) : null;
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
          setFileError('Drop one file at a time. No files were imported.');
          return;
        }
        const first = paths[0];
        if (!first) return;
        setSourcePath(first);
        setFileError(null);
        const name = first.split(/[\\/]/).at(-1);
        if (name) setPath(`/documents/${name}`);
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
  }, [bridge, itemKind]);

  const submit = async (): Promise<void> => {
    if (!store || !canWrite || saving || !path.startsWith('/')) return;
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
      } else if (typed.code === 'inactive-group') {
        try {
          await onApplied(`${store.name} is inactive`);
          setWorkflow(null);
        } catch (refreshError) {
          onError(refreshError);
        }
      } else onError(error);
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
          ? 'A login with a masked password.'
          : itemKind === 'Resource'
            ? 'A value such as an API key or recovery code.'
            : itemKind === 'File'
              ? 'A file streamed from its path by the local agent.'
              : 'A path pointing to another path in the same store.'
      }
      footer={
        <>
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!canWrite || saving}
            onClick={() => void submit()}
          >
            {saving
              ? 'Creating…'
              : itemKind === 'File' && !sourcePath
                ? 'Choose file and create'
                : 'Create in this vault'}
          </Button>
        </>
      }
    >
      <>
        <SectionLabel>Save in</SectionLabel>
        <Inset>
          {world.stores.length ? (
            <CardSelect
              label="Save in"
              options={storeNavigationOrder(world).map((candidate) =>
                storeOption(world, candidate),
              )}
              value={storeId}
              onChange={setStoreId}
              placeholder="Choose a vault"
            />
          ) : (
            <InsetRow label="Vault">
              <span className="dim">No vault is available to save into.</span>
            </InsetRow>
          )}
        </Inset>
        {store && blocked ? <Band>{`${store.name}: ${blocked}`}</Band> : null}
        {store ? (
          <AccessBlock
            world={world}
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
                  setPath(`/logins/${next}`);
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
              {field('Path', path, setPath, DRAFT_PATH.Password)}
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
                      `/agents/${next.toLowerCase().replace(/[^a-z0-9._-]+/g, '-')}`,
                    );
                },
                'e.g. ANTHROPIC_API_KEY',
              )}
              {field('Value', value, setValue, 'sk-ant-…', true)}
              {field('Path', path, setPath, DRAFT_PATH.Resource)}
            </>
          ) : null}
          {itemKind === 'Link' ? (
            <>
              {field('Points to', target, setTarget, '/ssh/id_ed25519', true)}
              {field('Path', path, setPath, DRAFT_PATH.Link)}
            </>
          ) : null}
          {itemKind === 'File' ? (
            <>
              <InsetRow label="File">
                <span className={sourcePath ? 'mono' : 'dim'}>
                  {sourcePath
                    ? sourcePath.split(/[\\/]/).at(-1)
                    : hovering
                      ? 'Drop to use this file'
                      : 'Drop a file here, or use the native picker'}
                </span>
              </InsetRow>
              {field('Path', path, setPath, DRAFT_PATH.File)}
            </>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
      </>
    </Sheet>
  );
}

function ExistsSheet({
  world,
  workflow,
  setWorkflow,
  onOpenExisting,
  onError,
}: {
  world: World;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const clash = world.items.find(
    (item) => item.store === workflow.storeId && item.path === workflow.path,
  );
  const version = clash?.version;
  return (
    <Sheet
      glyph={
        clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : undefined
      }
      title={`Something is already at ${workflow.path}`}
      subtitle={`New ${kindLabel(workflow.itemKind).toLowerCase()} · not created`}
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
            Change the path
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
            Open{version ? ` version ${version}` : ' existing item'}
          </Button>
        </>
      }
    >
      <>
        <p>
          Nothing was created and nothing was overwritten. Creating carries
          “must not exist”
          {version
            ? `, and ${nameOf(workflow.path)} was listed at version ${version} in ${storeOf(world, workflow.storeId)?.name ?? ''}`
            : ''}
          .
        </p>
        <p className="fn">
          Refresh and open what is there to review its exact version, or save
          this one at another path. There is no “create anyway”.
        </p>
      </>
    </Sheet>
  );
}

export function WriteOverlay({
  world,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onError,
  onRefreshConflict,
  onDiscardConflict,
  onOpenExisting,
}: {
  world: World;
  bridge: Bridge;
  workflow: WriteWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown, item?: Item) => void;
  onRefreshConflict: (item: Item, draft: string) => Promise<void>;
  onDiscardConflict: () => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
}): ReactNode {
  if (!workflow) return null;
  if (workflow.kind === 'agent-lost')
    return (
      <Dialog
        className="stopwrap"
        role="alertdialog"
        aria-label="Local agent connection lost"
      >
        <div className="notice stop">
          <div className="who">This Mac · the local agent</div>
          <h2>The local agent connection was lost</h2>
          <p>
            We encountered a connection error with the local FOKS agent. Your
            data has been saved, and in-progress operations can be continued
            once a connection is restored.
          </p>
          <div className="acts2">
            <Button
              variant="primary"
              onClick={() => {
                void (async () => {
                  try {
                    await bridge.retryAgentConnection();
                    await onApplied('Connected to the local agent');
                    setWorkflow(null);
                  } catch (error) {
                    onError(error);
                  }
                })();
              }}
            >
              Retry
            </Button>
          </div>
        </div>
      </Dialog>
    );
  if (workflow.kind === 'new')
    return (
      <DismissibleDialog
        className="backdrop"
        aria-label={`New ${kindLabel(workflow.itemKind).toLowerCase()}`}
        onDismiss={() => setWorkflow(null)}
      >
        <NewSheet
          world={world}
          bridge={bridge}
          workflow={workflow}
          setWorkflow={setWorkflow}
          onApplied={onApplied}
          onError={(error) => onError(error)}
        />
      </DismissibleDialog>
    );
  if (workflow.kind === 'exists')
    return (
      <DismissibleDialog
        className="backdrop"
        aria-label="Creation refused because the path exists"
        onDismiss={() => setWorkflow(null)}
      >
        <ExistsSheet
          world={world}
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
    <RemoveSheet
      workflow={workflow}
      bridge={bridge}
      setWorkflow={setWorkflow}
      onApplied={onApplied}
      onError={onError}
    />
  );
}

/**
 * The save-refused sheet.
 *
 * Escape used to be wired straight to "discard my edit" here, while every
 * other overlay in the app treats Escape as a harmless cancel — and the body
 * of this very sheet promises the draft is retained. Pressing the most
 * reflexive key in the window therefore destroyed work the sheet had just
 * said it was keeping. Escape now asks; the discard itself is still one
 * deliberate click away in the footer.
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
        aria-label="Discard your edit"
        onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
          // Escape backs out of the question, not out of the draft.
          if (event.key === 'Escape') setConfirmingDiscard(false);
        }}
      >
        <Sheet
          glyph={
            <span className="kico md danger">
              <Icon name="trash" />
            </span>
          }
          title="Discard your edit?"
          subtitle={`${nameOf(workflow.item.path)} · not saved anywhere`}
          footer={
            <>
              <Button onClick={() => setConfirmingDiscard(false)}>
                Keep editing
              </Button>
              <Button
                variant="primary"
                className="danger"
                onClick={onDiscardConflict}
              >
                Discard my edit
              </Button>
            </>
          }
        >
          <p>
            Your draft has not been saved. Discarding it here is the only copy
            gone — the item itself is untouched at its current version.
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
        title="Someone else changed this first"
        subtitle={`${nameOf(workflow.item.path)} · save refused`}
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
            You edited version {workflow.item.version}, but that exact version
            is no longer current, so nothing was saved and nothing was
            overwritten.
          </p>
          <p className="fn">
            Refresh to see the current version beside your retained draft,
            review it, and save again under the refreshed version. There is no
            “save anyway” and Retry never replays this write.
          </p>
        </>
      </Sheet>
    </Dialog>
  );
}

/**
 * The remove confirmation.
 *
 * Unlike every other write on this screen it had no busy flag, so the natural
 * response to a slow remove — clicking again — sent a second exact-version
 * remove. The item was already gone, so that one came back as a *conflict*,
 * and the user was shown "Someone else changed this first" with a retained
 * draft that never existed, for an item that no longer did.
 */
function RemoveSheet({
  workflow,
  bridge,
  setWorkflow,
  onApplied,
  onError,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'remove' }>;
  bridge: Bridge;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown, item?: Item) => void;
}): ReactNode {
  const [removing, setRemoving] = useState(false);
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      title={`Remove ${nameOf(workflow.item.path)}?`}
      subtitle={workflow.item.path}
      footer={
        <>
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            className="danger"
            variant="primary"
            disabled={removing}
            onClick={() => {
              if (removing) return;
              setRemoving(true);
              void (async () => {
                try {
                  await bridge.removeItem({
                    storeId: workflow.item.store,
                    path: workflow.item.path,
                    version: workflow.item.version,
                  });
                  await onApplied(`Removed version ${workflow.item.version}`);
                  setWorkflow(null);
                } catch (error) {
                  onError(error, workflow.item);
                } finally {
                  setRemoving(false);
                }
              })();
            }}
          >
            {removing ? 'Removing…' : 'Remove'}
          </Button>
        </>
      }
    >
      <p>This will remove the item. This can’t be undone.</p>
    </SheetDialog>
  );
}
