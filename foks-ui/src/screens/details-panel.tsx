/**
 * The details panel — `01-vault.html`'s `renderDetails()`, extended with the
 * exact-version item actions that landed after the frozen mock.
 *
 * It says what an item *is*: where it lives, what kind of node it is a
 * reading of, its version and size, the roles that gate reading and writing
 * it, and — computed, never typed — who else can read it. Everything on it is
 * metadata the agent already answers.
 *
 * The value stays **masked** until Show. Show is the one call that returns a
 * secret to the webview and is guarded in Rust by the app lock. Copy and
 * Download execute entirely in Rust. Edit and Remove carry ExactVersion and
 * are enabled only when the authenticated account or group role admits the
 * listed write role.
 *
 * A link target is learned by the same exact-version content read, then Open
 * target changes the selection within the same store.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useToast } from '/kit/toasts';
import type { ReactNode } from 'react';
import {
  Avatar,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  SectionLabel,
  Toggle,
} from '../components';
import type { FilterKind } from '../components';
import {
  canChangeItem,
  catalog,
  fmtSize,
  formatRole,
  kindOf,
  nameOf,
  parseRole,
  partiesOf,
  partyName,
  peopleLabel,
  readersOf,
  rtype,
  serverOf,
  shortId,
  storeOf,
} from '../model';
import type { Item, Party, RoleWire, World } from '../model';
import type { Selection } from '../location';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemRequest, ReadItemResponse } from '../bridge';
import type { MutationFailureHandler } from '../mutation-recovery';
import { editableValue } from './edit-value';

const FIELD_LABELS: Readonly<Record<string, string>> = {
  user: 'User name',
  password: 'Password',
  url: 'Website',
  ssid: 'Network',
};

/** StrictMode can replay mount effects; coalesce only the in-flight read. */
const readFlights = new WeakMap<
  Bridge,
  Map<string, Promise<ReadItemResponse>>
>();

function readOnce(
  bridge: Bridge,
  request: ItemRequest,
): Promise<ReadItemResponse> {
  const key = JSON.stringify([request.storeId, request.path, request.version]);
  const flights =
    readFlights.get(bridge) ?? new Map<string, Promise<ReadItemResponse>>();
  readFlights.set(bridge, flights);
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
      'The local agent returned a value for a different catalog selection.',
    );
  }
}

/** "Owner" / "Member · visibility 0", from either shape of the wire. */
function roleText(wire: RoleWire): string {
  const role = parseRole(wire);
  if (role) return formatRole(role);
  return typeof wire === 'string' ? wire : wire.role;
}

/** The wire shape of a role: Member carries a band, Admin and Owner do not. */
function wireRole(wire: RoleWire): { role: string; visibility?: number } {
  const role = parseRole(wire);
  if (!role) return { role: typeof wire === 'string' ? wire : wire.role };
  if (role.kind === 'member')
    return { role: 'Member', visibility: role.visibility ?? 0 };
  return { role: role.kind === 'admin' ? 'Admin' : 'Owner' };
}

/** `user: rae` → `['user', 'rae']`; a line with no field is all value. */
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
 * Fixture rows carry safe masked context; native catalog rows carry none.
 * After Show, a structured password record can introduce its own fields. An
 * unstructured read remains one Value row rather than being mislabeled.
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

function PartyRow({
  party,
  canRead,
  serverName,
}: {
  party: Party;
  canRead: boolean;
  serverName: string;
}): ReactNode {
  return (
    <div className="party">
      <Avatar party={party} className="pav" />
      <span className="t">
        {partyName(party)}
        {party.label ? (
          <>
            {' '}
            <Chip tone="you">you</Chip>
          </>
        ) : null}
        <small>
          {party.note ??
            (party.party_kind !== 'user' ? 'a member group' : serverName)}
          {' · '}
          {shortId(party.party_id_hex)} · gen {party.generation}
          {canRead ? '' : ' · cannot read this'}
        </small>
      </span>
      <Chip>{roleText(party.destination_role)}</Chip>
    </div>
  );
}

export interface DetailsPanelProps {
  world: World;
  bridge: Bridge;
  selection: Selection;
  /** `<store>|<path>` when a row or the `show` scene requested Show. */
  revealRequest?: string | null;
  onRevealHandled?: () => void;
  onSelect: (selection: Selection) => void;
  onClose: () => void;
  onRemove: (item: Item) => void;
  onConflict: (item: Item, draft: string) => void;
  onApplied: (message: string) => Promise<void>;
  onCommandError: (error: unknown, item?: Item) => void;
  onMutationError: MutationFailureHandler;
  /** Increments when the agent is lost so every renderer-held value is dropped. */
  concealSignal?: number;
  resumeDraft?: {
    store: string;
    path: string;
    value: string;
    epoch: number;
  } | null;
}

export function DetailsPanel({
  world,
  bridge,
  selection,
  revealRequest = null,
  onRevealHandled,
  onSelect,
  onClose,
  onRemove,
  onConflict,
  onApplied,
  onCommandError,
  onMutationError,
  concealSignal = 0,
  resumeDraft = null,
}: DetailsPanelProps): ReactNode {
  const item: Item | undefined = selection
    ? world.items.find(
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
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState('');
  const [saving, setSaving] = useState(false);
  const [replacementPath, setReplacementPath] = useState<string | null>(null);
  const [dropHover, setDropHover] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);
  const [binaryFile, setBinaryFile] = useState(false);
  const concealEpoch = useRef(0);
  // The selection an async read was started for, readable from inside a
  // promise callback without closing over a stale render's `key`.
  // The epoch of the retained draft already restored, so a later refresh
  // does not restore it a second time.
  const appliedDraft = useRef<number | null>(null);
  const keyRef = useRef(key);
  keyRef.current = key;

  const request = useMemo<ItemRequest | null>(
    () =>
      item
        ? { storeId: item.store, path: item.path, version: item.version }
        : null,
    [item],
  );

  const show = useCallback(async () => {
    if (!item || !request) return;
    const requestKey = key;
    const epoch = concealEpoch.current;
    setRead({ key: requestKey, state: 'loading' });
    try {
      const response = await readOnce(bridge, request);
      assertExactRead(request, response);
      if (epoch !== concealEpoch.current) return;
      setRead({ key: requestKey, state: 'shown', value: response.value });
    } catch (error) {
      if (epoch !== concealEpoch.current) return;
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
  }, [bridge, item, key, onCommandError, request]);

  useEffect(() => {
    concealEpoch.current += 1;
    setRead(null);
    setEditing(false);
    setEditValue('');
    setReplacementPath(null);
    setEditError(null);
    setBinaryFile(false);
  }, [key]);

  // Show is an explicit action. Selection alone never reads content, including
  // a link target, which is not catalog metadata.
  useEffect(() => {
    if (!item) return;
    const requested = revealRequest === `${item.store}|${item.path}`;
    if (!requested) return;
    onRevealHandled?.();
    if (read?.key !== key) void show();
  }, [item, key, onRevealHandled, read?.key, revealRequest, show]);

  // JS strings cannot be zeroized. Dropping the only reference on blur is the
  // strongest webview-side bound; Rust-side copy avoids creating another.
  useEffect(() => {
    const conceal = (): void => {
      concealEpoch.current += 1;
      setRead(null);
    };
    window.addEventListener('blur', conceal);
    return () => window.removeEventListener('blur', conceal);
  }, [item]);

  useEffect(() => {
    if (!concealSignal) return;
    concealEpoch.current += 1;
    setRead(null);
    setEditValue('');
    setEditing(false);
    setReplacementPath(null);
  }, [concealSignal]);

  useEffect(() => {
    if (!item || !resumeDraft) return;
    if (item.store !== resumeDraft.store || item.path !== resumeDraft.path)
      return;
    // A retained draft is restored once, for the refresh that produced it.
    // `item` is a fresh object after every `loadWorld`, so without this the
    // effect re-ran on each catalog read and pushed the pre-conflict draft
    // back into the editor — over a version the user had since saved, and
    // back onto the screen after a conceal had cleared it.
    if (appliedDraft.current === resumeDraft.epoch) return;
    appliedDraft.current = resumeDraft.epoch;
    setEditValue(resumeDraft.value);
    setEditing(true);
  }, [item, resumeDraft]);

  const selectedKind = item ? kindOf(item) : null;
  const selectedFileMode = selectedKind === 'File' || binaryFile;
  useEffect(() => {
    if (!editing || !selectedFileMode) return;
    let disposed = false;
    let hoverOff: (() => void) | undefined;
    let pathsOff: (() => void) | undefined;
    void bridge
      .onDropHover(({ hovering }) => setDropHover(hovering))
      .then(
        (off) => {
          if (disposed) off();
          else hoverOff = off;
        },
        (error) => {
          if (!disposed) setEditError(normalizeCommandError(error).message);
        },
      );
    void bridge
      .onDropPaths((paths) => {
        if (paths.length !== 1) {
          setReplacementPath(null);
          setEditError(
            'Drop one file at a time. The original file was not changed.',
          );
        } else {
          setReplacementPath(paths[0] ?? null);
          setEditError(null);
        }
        setDropHover(false);
      })
      .then(
        (off) => {
          if (disposed) off();
          else pathsOff = off;
        },
        (error) => {
          if (!disposed) setEditError(normalizeCommandError(error).message);
        },
      );
    return () => {
      disposed = true;
      hoverOff?.();
      pathsOff?.();
    };
  }, [bridge, editing, selectedFileMode]);

  if (!item) {
    return (
      <aside className="details" aria-label="Details">
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
            <Icon name="x" />
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

  const store = storeOf(world, item.store);
  const server = serverOf(world, item.store);
  const serverName = server?.name ?? '';
  const kind = kindOf(item) as FilterKind;
  const fileMode = kind === 'File' || binaryFile;
  const displayKind: FilterKind = fileMode ? 'File' : kind;
  const team = store?.kind === 'team';
  const canChange = canChangeItem(world, item);
  const readers = readersOf(world, item);
  const parties = store ? partiesOf(world, store.id) : [];
  const activeRead = read?.key === key ? read : null;
  const shownValue = activeRead?.state === 'shown' ? activeRead.value : null;
  const readError = activeRead?.state === 'error' ? activeRead.message : null;
  const reading = activeRead?.state === 'loading';
  const copyValue = async (): Promise<void> => {
    if (!request) return;
    try {
      await bridge.copyItemValue(request);
      toasts.show(`${kind === 'Password' ? 'Password' : 'Value'} copied`);
    } catch (error) {
      if (normalizeCommandError(error).code === 'agent-lost')
        onCommandError(error, item);
      else
        toasts.show(normalizeCommandError(error).message, { tone: 'warning' });
    }
  };

  const beginEdit = async (): Promise<void> => {
    if (!request) return;
    if (fileMode) {
      setEditing(true);
      return;
    }
    // The same bound `show` keeps. `assertExactRead` only proves the response
    // matches the request that was sent; it says nothing about what is
    // selected by the time it lands. Without this, switching rows while a
    // slow read is in flight opens the *new* item's editor holding the *old*
    // item's plaintext — and Save then writes it under the new item's version.
    const requestKey = key;
    const epoch = concealEpoch.current;
    const current = (): boolean =>
      epoch === concealEpoch.current && requestKey === keyRef.current;
    setSaving(true);
    try {
      const response = await readOnce(bridge, request);
      assertExactRead(request, response);
      if (!current()) return;
      setEditValue(editableValue(item, response.value));
      setEditing(true);
      setRead(null);
    } catch (error) {
      if (!current()) return;
      const typed = normalizeCommandError(error);
      if (typed.code === 'not-text' && item.kind === 'Secret') {
        setBinaryFile(true);
        setRead(null);
        setEditing(true);
      } else {
        onCommandError(error, item);
      }
    } finally {
      // `saving` is shared with the row that is on screen now, so it is
      // always cleared — otherwise a late read leaves the new item's Edit
      // button disabled and reading "Reading…".
      setSaving(false);
    }
  };

  const saveEdit = async (): Promise<void> => {
    if (!request) return;
    setSaving(true);
    try {
      if (fileMode) {
        const response = replacementPath
          ? await bridge.replaceDroppedFile({
              ...request,
              sourcePath: replacementPath,
            })
          : await bridge.pickAndReplaceFile(request);
        if (!response.applied) return;
      } else {
        await bridge.editTextItem({ ...request, value: editValue });
      }
      setEditing(false);
      await onApplied(`Saved version ${item.version + 1}`);
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'conflict') {
        onConflict(item, editValue);
        await onMutationError(error, { report: false });
      } else {
        await onMutationError(error, { item });
      }
    } finally {
      setSaving(false);
    }
  };

  const preview =
    editing && fileMode ? (
      <Inset variant="preview" className={dropHover ? 'drop-hover' : undefined}>
        <InsetRow label="Replace">
          <span className={replacementPath ? 'mono' : 'dim'}>
            {replacementPath
              ? replacementPath.split(/[\\/]/).at(-1)
              : dropHover
                ? 'Drop to use this file'
                : 'Drop a file here, or use the native picker when saving'}
          </span>
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
                className={shownValue === null ? undefined : 'rev'}
                valueClass={shownValue === null ? 'mask' : 'mono'}
                action={
                  shownValue === null ? (
                    <>
                      <button
                        type="button"
                        disabled={reading}
                        onClick={() => void show()}
                      >
                        <Icon name="eye" />
                        {reading ? 'Reading…' : 'Show'}
                      </button>
                      <button type="button" onClick={() => void copyValue()}>
                        <Icon name="copy" />
                        Copy
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        type="button"
                        onClick={() => {
                          concealEpoch.current += 1;
                          setRead(null);
                        }}
                      >
                        <Icon name="eyeoff" />
                        Hide
                      </button>
                      <button type="button" onClick={() => void copyValue()}>
                        <Icon name="copy" />
                        Copy
                      </button>
                    </>
                  )
                }
              >
                {value}
              </InsetRow>
            ) : (
              <InsetRow key={index} label={FIELD_LABELS[field ?? ''] ?? field}>
                {value}
              </InsetRow>
            ),
        )}
      </Inset>
    ) : kind === 'Resource' && !fileMode ? (
      <Inset variant="preview">
        <InsetRow
          className="rev"
          label="Value"
          valueClass={shownValue === null ? 'mask' : 'mono'}
          action={
            shownValue === null ? (
              <>
                <button
                  type="button"
                  disabled={reading}
                  onClick={() => void show()}
                >
                  <Icon name="eye" />
                  {reading ? 'Reading…' : 'Show'}
                </button>
                <button type="button" onClick={() => void copyValue()}>
                  <Icon name="copy" />
                  Copy
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  onClick={() => {
                    concealEpoch.current += 1;
                    setRead(null);
                  }}
                >
                  <Icon name="eyeoff" />
                  Hide
                </button>
                <button type="button" onClick={() => void copyValue()}>
                  <Icon name="copy" />
                  Copy
                </button>
              </>
            )
          }
        >
          {shownValue ?? '••••••••••••••••••••'}
        </InsetRow>
      </Inset>
    ) : fileMode ? (
      <Inset variant="preview">
        <div className="pad">
          <div className="fileglyph">
            <span className="g">
              <Icon name="file" />
            </span>
            <span>
              <b>{nameOf(item.path)}</b>
              <span>
                {fmtSize(item.size)} · version {item.version}
              </span>
            </span>
          </div>
          <div className="row2">
            <Button
              variant="primary"
              icon="download"
              onClick={() => {
                if (!request) return;
                void bridge.downloadFile(request).then(
                  ({ saved }) =>
                    toasts.show(
                      saved
                        ? `Downloaded version ${item.version}`
                        : 'Download cancelled',
                    ),
                  (error) => {
                    if (normalizeCommandError(error).code === 'agent-lost')
                      onCommandError(error, item);
                    else
                      toasts.show(normalizeCommandError(error).message, {
                        tone: 'warning',
                      });
                  },
                );
              }}
            >
              Download
            </Button>
          </div>
        </div>
      </Inset>
    ) : (
      <Inset variant="preview">
        <div className="pad">
          <div className="fileglyph">
            <span className="g Link">
              <Icon name="link" />
            </span>
            <span>
              <b>{nameOf(item.path)}</b>
              <span>
                {shownValue === null ? (
                  reading ? (
                    'reading the version-bound target…'
                  ) : (
                    'target masked until read'
                  )
                ) : (
                  <>
                    points to <code>{shownValue}</code>
                  </>
                )}
              </span>
            </span>
          </div>
          <div className="row2">
            <Button
              variant="primary"
              icon="arrow"
              disabled={reading}
              onClick={() => {
                if (shownValue === null) {
                  void show();
                  return;
                }
                // Through the catalog, not the raw item list: the raw list
                // still holds folders, and selecting one reaches KindIcon
                // with a kind it does not draw.
                const target = catalog(world).find(
                  (candidate) =>
                    candidate.store === item.store &&
                    candidate.path === shownValue,
                );
                if (target)
                  onSelect({ store: target.store, path: target.path });
              }}
            >
              {shownValue === null
                ? reading
                  ? 'Reading…'
                  : 'Read target'
                : 'Open target'}
            </Button>
          </div>
        </div>
      </Inset>
    );

  const footnote =
    kind === 'Link'
      ? shownValue === null
        ? `Read target loads the link from version ${item.version}. FOKS hides it again when the window loses focus.`
        : `Version ${item.version} links to ${shownValue} on ${serverName}. Opening it reads the current item at that path.`
      : '';

  return (
    <aside className="details" aria-label={`Details for ${nameOf(item.path)}`}>
      <div className="dh">
        <KindIcon kind={displayKind} />
        <span className="t">
          <h2>{nameOf(item.path)}</h2>
          <small>
            {displayKind} in {store?.name}
          </small>
        </span>
        <button
          type="button"
          className="x"
          title="Close"
          aria-label="Close"
          onClick={onClose}
        >
          <Icon name="x" />
        </button>
      </div>
      <div className="scroll">
        {editing || displayKind !== 'Resource' ? (
          <SectionLabel>
            {editing
              ? 'Edit'
              : displayKind === 'Password'
                ? 'Login'
                : displayKind}
          </SectionLabel>
        ) : null}
        {preview}
        {editing || footnote ? (
          <div className="pfn">
            {editing
              ? `Save succeeds only if this item is still version ${item.version}. If it changed, refresh it and try again.`
              : footnote}
          </div>
        ) : null}
        {readError ? (
          <p className="action-error" role="alert">
            {readError}
          </p>
        ) : null}
        {editError ? (
          <p className="action-error" role="alert">
            {editError}
          </p>
        ) : null}

        <SectionLabel>Info</SectionLabel>
        <div className="meta">
          <b>Path</b>
          <code>{item.path}</code>
          <b>Kind</b>
          <span>{displayKind}</span>
          <b>Version</b>
          <span>{item.version}</span>
          <b>Size</b>
          <span>{fmtSize(item.size)}</span>
          <b>Read role</b>
          <span>
            <Chip>{roleText(item.read)}</Chip>
          </span>
          <b>Write role</b>
          <span>
            <Chip>{roleText(item.write)}</Chip>
          </span>
        </div>

        <div className="who">
          <SectionLabel>Sharing</SectionLabel>
          {team && readers ? (
            <>
              <p>
                {peopleLabel(readers.length)} can read this — everyone in{' '}
                {store?.name} at <b>{roleText(item.read)}</b> or above.
              </p>
              {parties.map((party) => (
                <PartyRow
                  key={party.party_id_hex}
                  party={party}
                  serverName={serverName}
                  canRead={readers.includes(party)}
                />
              ))}
            </>
          ) : (
            <p>Only you</p>
          )}
        </div>

        <Toggle label="Inspect response">
          <pre>
            {JSON.stringify(
              {
                path: item.path,
                node_type: rtype(item),
                version: item.version,
                size: item.size,
                read_role: wireRole(item.read),
                write_role: wireRole(item.write),
              },
              null,
              1,
            )}
          </pre>
        </Toggle>
      </div>
      <div className="dfoot">
        {editing ? (
          <>
            <Button disabled={saving} onClick={() => setEditing(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={saving}
              onClick={() => void saveEdit()}
            >
              {saving ? 'Saving…' : `Save version ${item.version + 1}`}
            </Button>
          </>
        ) : (
          <>
            <Button
              icon="pencil"
              disabled={!canChange || kind === 'Link' || saving}
              title={
                !canChange
                  ? `You need ${roleText(item.write)} permissions to edit this item.`
                  : kind === 'Link'
                    ? 'Links cannot be edited directly. Delete this link and create a new one.'
                    : `Edit this item (version ${item.version})`
              }
              onClick={() => void beginEdit()}
            >
              {saving ? 'Reading…' : 'Edit'}
            </Button>
            <Button
              onClick={() => {
                if (!request) return;
                void bridge.copyItemPath(request).then(
                  () => toasts.show(`Path copied: ${item.path}`),
                  (error) => {
                    if (normalizeCommandError(error).code === 'agent-lost')
                      onCommandError(error, item);
                    else
                      toasts.show(normalizeCommandError(error).message, {
                        tone: 'warning',
                      });
                  },
                );
              }}
            >
              Copy path
            </Button>
            <Button
              variant="danger"
              icon="trash"
              disabled={!canChange}
              title={
                canChange
                  ? 'Delete this item'
                  : `You need ${roleText(item.write)} permissions to delete this item.`
              }
              onClick={() => onRemove(item)}
            >
              Remove
            </Button>
          </>
        )}
      </div>
    </aside>
  );
}
