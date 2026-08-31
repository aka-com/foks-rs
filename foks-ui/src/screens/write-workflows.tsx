import { useEffect, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Dialog } from '/kit/overlay-primitives';
import {
  Button,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  Stack,
} from '../components';
import type { FilterKind } from '../components';
import {
  isLogin,
  itemKey,
  kindOf,
  canCreateInStore,
  leaseLapsed,
  nameOf,
  partiesOf,
  partyName,
  peopleGroups,
  readersOf,
  serverBlocked,
  serverLeaseUnavailable,
  serverOf,
  storeOf,
} from '../model';
import type { Item, ItemKind, RoleWire, Store, StoreRef, World } from '../model';
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

function StoreChoice({
  world,
  store,
  chosen,
  onChoose,
}: {
  world: World;
  store: Store;
  chosen: boolean;
  onChoose: () => void;
}): ReactNode {
  const server = serverOf(world, store.id);
  const unavailable = !canCreateInStore(world, store.id);
  const detail = leaseLapsed(world, store.id)
    ? 'server check-in lapsed — nothing can be written here'
    : serverBlocked(world, store.id)
      ? 'server access is blocked — nothing can be written here'
      : serverLeaseUnavailable(world, store.id)
        ? 'signed check-in unavailable — nothing can be written here'
      : store.kind === 'team'
        ? store.active && unavailable
          ? 'no authenticated local group identity — writing is unavailable'
          : store.active
          ? `shared with ${peopleGroups(partiesOf(world, store.id))} · ${server?.name ?? ''}`
          : 'reports inactive — resume its creation first'
        : `only you · your account on ${server?.name ?? ''}`;
  return (
    <button
      type="button"
      className={`radio store-choice${chosen ? ' on' : ''}${unavailable ? ' off' : ''}`}
      disabled={unavailable}
      onClick={onChoose}
    >
      <span className="rb" />
      <span className="t">
        <b>{store.name}</b>
        <small>{detail}</small>
      </span>
      {store.kind === 'team' ? (
        <Stack parties={partiesOf(world, store.id)} />
      ) : null}
    </button>
  );
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
  const changers = readersOf(world, { ...candidate, read: candidate.write }) ?? [];
  const readVisibility = readRole.startsWith('Member:')
    ? Number(readRole.slice('Member:'.length))
    : 0;
  const choices: readonly [string, KvRoleInput, string][] = [
    ['Owner', 'Owner', 'Only owners.'],
    ['Admin', 'Admin', 'Admins and owners.'],
    ['Member', `Member:${readVisibility}`, 'Members at this visibility level and above.'],
  ];
  const roleChoices = (
    selected: KvRoleInput,
    choose: (role: KvRoleInput) => void,
    forWrite: boolean,
  ) => choices.map(([label, role, detail]) => {
    const next = label === 'Member'
      ? (`Member:${selected.startsWith('Member:') ? Number(selected.slice('Member:'.length)) : 0}` as KvRoleInput)
      : role;
    const on = selected === next || (label === 'Member' && selected.startsWith('Member:'));
    return (
      <button
        type="button"
        className={`radio${on ? ' on' : ''}`}
        key={label}
        aria-label={`${forWrite ? 'Write' : 'Read'} role ${label}`}
        aria-pressed={on}
        onClick={() => choose(next)}
      >
        <span className="rb" />
        <span className="t"><b>{label}</b><small>{forWrite ? detail.replace('.', ' can change or remove it.') : detail}</small></span>
      </button>
    );
  });
  return (
    <>
      <div className="sec">
        Who can read
        <span className="pv">
          would be readable by{' '}
          <b>
            {admitted.length} of {roster.length}
          </b>
        </span>
      </div>
      <Inset>
        {roleChoices(readRole, onReadRole, false)}
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
        At <b>{readRole === 'Owner' || readRole === 'Admin' ? readRole : `Member · visibility ${readVisibility}`}</b> this item would be readable by{' '}
        <b>
          {admitted.length} of {roster.length}
        </b>{' '}
        in {store.name}: {admitted.map(partyName).join(', ') || 'nobody'}.
        {excluded.length
          ? ` ${excluded.map(partyName).join(', ')} is not counted when its role or group membership provides no access here.`
          : ''}
      </p>
      <div className="sec">Who can change <span className="pv">changeable by <b>{changers.length} of {roster.length}</b></span></div>
      <Inset>
        {roleChoices(writeRole, onWriteRole, true)}
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
        The read and write roles are independent and carried by this group
        item, so someone allowed to change it might not be allowed to read it.
        They are checked against the current authenticated roster, so the
        preview is computed rather than typed.
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
        await bridge.createTextItem({ storeId: store.id, path, value, ...roleArgs });
      } else if (itemKind === 'Link') {
        await bridge.createLink({ storeId: store.id, path, target, ...roleArgs });
      } else if (sourcePath) {
        await bridge.importDroppedFile({ storeId: store.id, path, sourcePath, ...roleArgs });
      } else {
        const result = await bridge.pickAndImportFile({
          storeId: store.id,
          path,
          ...roleArgs,
        });
        if (!result.applied) return;
      }
      await onApplied(`${itemKind} created in ${store.name}`);
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
    <InsetRow label={label} valueClass={mono ? 'mono' : undefined}>
      <input
        type={type}
        aria-label={label}
        className={mono ? 'mono' : undefined}
        placeholder={placeholder}
        value={current}
        onChange={(event) => set(event.target.value)}
      />
    </InsetRow>
  );
  return (
    <div className="sheet mid" aria-label={`New ${itemKind.toLowerCase()}`}>
      <div className="hd">
        <KindIcon kind={itemKind} />
        <span className="t">
          <h2>New {itemKind.toLowerCase()}</h2>
          <small>
            {itemKind === 'Password'
              ? 'A login with a masked password.'
              : itemKind === 'Resource'
                ? 'A value such as an API key or recovery code.'
                : itemKind === 'File'
                  ? 'A file streamed from its path by the local agent.'
                  : 'A path pointing to another path in the same store.'}
          </small>
        </span>
      </div>
      <div className="sb">
        <div className="sec">Save in</div>
        <Inset>
          {world.stores.map((candidate) => (
            <StoreChoice
              key={candidate.id}
              world={world}
              store={candidate}
              chosen={candidate.id === storeId}
              onChoose={() => setStoreId(candidate.id)}
            />
          ))}
        </Inset>
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
        <div className="sec">{itemKind}</div>
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
              {field('User name', username, setUsername, 'rae')}
              {field(
                'Password',
                password,
                setPassword,
                'Password',
                true,
                'password',
              )}
              {field(
                'Website',
                website,
                setWebsite,
                'https://github.com/login',
              )}
              {field('Path', path, setPath, DRAFT_PATH.Password, true)}
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
              {field('Path', path, setPath, DRAFT_PATH.Resource, true)}
            </>
          ) : null}
          {itemKind === 'Link' ? (
            <>
              {field('Points to', target, setTarget, '/ssh/id_ed25519', true)}
              {field('Path', path, setPath, DRAFT_PATH.Link, true)}
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
              {field('Path', path, setPath, DRAFT_PATH.File, true)}
            </>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
      </div>
      <div className="ft">
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
              : `Create in ${store?.name ?? ''}`}
        </Button>
      </div>
    </div>
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
    <div className="sheet">
      <div className="hd">
        {clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : null}
        <span className="t">
          <h2>Something is already at {workflow.path}</h2>
          <small>New {workflow.itemKind.toLowerCase()} · not created</small>
        </span>
      </div>
      <div className="sb">
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
      </div>
      <div className="ft">
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
      </div>
    </div>
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
  const dismiss = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key !== 'Escape') return;
    if (workflow.kind === 'conflict') onDiscardConflict();
    else setWorkflow(null);
  };
  if (workflow.kind === 'new')
    return (
      <Dialog
        className="backdrop"
        aria-label={`New ${workflow.itemKind.toLowerCase()}`}
        onKeyDown={dismiss}
      >
        <NewSheet
          world={world}
          bridge={bridge}
          workflow={workflow}
          setWorkflow={setWorkflow}
          onApplied={onApplied}
          onError={(error) => onError(error)}
        />
      </Dialog>
    );
  if (workflow.kind === 'exists')
    return (
      <Dialog
        className="backdrop"
        aria-label="Creation refused because the path exists"
        onKeyDown={dismiss}
      >
        <ExistsSheet
          world={world}
          workflow={workflow}
          setWorkflow={setWorkflow}
          onOpenExisting={onOpenExisting}
          onError={(error) => onError(error)}
        />
      </Dialog>
    );
  if (workflow.kind === 'conflict')
    return (
      <Dialog
        className="backdrop"
        aria-label="Edit conflict"
        onKeyDown={dismiss}
      >
        <div className="sheet">
          <div className="hd">
            <KindIcon kind={kindOf(workflow.item) as FilterKind} />
            <span className="t">
              <h2>Someone else changed this first</h2>
              <small>{nameOf(workflow.item.path)} · save refused</small>
            </span>
          </div>
          <div className="sb">
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
          </div>
          <div className="ft">
            <Button onClick={onDiscardConflict}>Discard my edit</Button>
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
          </div>
        </div>
      </Dialog>
    );
  return (
    <Dialog
      className="backdrop"
      role="alertdialog"
      aria-label={`Remove ${nameOf(workflow.item.path)}`}
      onKeyDown={dismiss}
    >
      <div className="sheet">
        <div className="hd">
          <span className="kico md danger">
            <Icon name="trash" />
          </span>
          <span className="t">
            <h2>Remove {nameOf(workflow.item.path)}?</h2>
            <small>{workflow.item.path}</small>
          </span>
        </div>
        <div className="sb">
          <p>This will remove the item. This can’t be undone.</p>
        </div>
        <div className="ft">
          <Button onClick={() => setWorkflow(null)}>Cancel</Button>
          <Button
            className="danger"
            variant="primary"
            onClick={() => {
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
                }
              })();
            }}
          >
            Remove
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
