/**
 * The mock host's vault commands, from `list_catalog` to `export_logins`.
 * Each answers with the JSON the native command returns, or throws the
 * CommandError it rejects with, so `bridge/commands-vault.ts` decodes and
 * normalizes it as it does the native host's answers.
 */

import type {
  CatalogDto,
  CommitPasswordImportRequest,
  CopyFieldRequest,
  CopyItemToStoreRequest,
  CreateFolderRequest,
  CreateTextRequest,
  EditTextRequest,
  EmptyTrashRequest,
  ImportDroppedFileRequest,
  ItemField,
  ItemRequest,
  KvRoleInput,
  MoveItemRequest,
  PermanentRemoveRequest,
  RemoveItemRequest,
  ReplaceDroppedFileRequest,
  RestoreTrashRequest,
} from '../bridge';
import { roleDto, normalizeCommandError } from '../bridge';
import type { TransportChannel } from '../bridge/transport';
import { storeReadable } from '../model/lease';
import { loginField, shapeOf } from '../model/login';
import { parseRole, roleRank, visibilityOf } from '../model/roles';
import {
  ORIGIN_SUFFIX,
  TRASH_ROOT,
  encodeOriginalPath,
  isTrashPath,
  parseTrashName,
  trashStamp,
} from '../model/trash';
import { itemKey } from '../model/types';
import type { Item, RoleWire } from '../model/types';
import { mockCommand, type MockCommand } from './host';
import { mockServerStatus } from './servers';
import { failure, type MockState } from './state';
import { accountWire, serverStatusWire } from './wire';

/** One clipboard write a mock bridge made, with the text the host would copy. */
export interface MockCopy {
  store: string;
  path: string;
  field: ItemField;
  text: string;
}

/** Every item copy made through a mock bridge, oldest first. */
export const mockCopies: MockCopy[] = [];

/** A mock item copy's response: the host's clear time outside Linux. */
const MOCK_CLIPBOARD_COPY = { ok: true, clearAfterSeconds: 30 } as const;

/** One website a mock bridge opened, with the address the host would open. */
export interface MockOpenedSite {
  store: string;
  path: string;
  url: string;
}

/** Every website opened through a mock bridge, oldest first. */
export const mockOpenedSites: MockOpenedSite[] = [];

/**
 * The address the host's `website_url` would open for a website field, or
 * `null` for one it refuses: a value whose text before its first `://` is not
 * a scheme is taken as https, and only an http or https address with a host
 * and no credentials is accepted.
 */
function mockWebsiteUrl(field: string): string | null {
  const trimmed = field.trim();
  if (/\s/.test(trimmed)) return null;
  try {
    const url = new URL(
      /^[A-Za-z][A-Za-z0-9+.-]*:\/\//.test(trimmed)
        ? trimmed
        : `https://${trimmed}`,
    );
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    if (!url.hostname || url.username || url.password) return null;
    return url.href;
  } catch {
    return null;
  }
}

const FIELD_NAMES: Readonly<Record<Exclude<ItemField, 'value'>, string>> = {
  password: 'password',
  username: 'user name',
  url: 'website',
};

/**
 * The file the mock's password import reads, as if from a Bitwarden CSV
 * export: two logins at the top, one of them named like a fixture login,
 * one in a folder, and a secure note that is left out. `text` is what the
 * host would write; the preview the renderer receives carries none of it.
 */
const MOCK_PASSWORD_IMPORT = {
  fileName: 'bitwarden_export.csv',
  entries: [
    {
      name: 'github.com',
      folder: '',
      text: 'user: satoshi\npassword: imported-gh\nurl: https://github.com',
    },
    {
      name: 'Mail',
      folder: 'Work',
      text: 'user: ana\npassword: imported-mail\nurl: https://mail.example.com\nnote: recovery codes are in the safe',
    },
    {
      name: 'Bank',
      folder: '',
      text: 'user: ana\npassword: imported-bank\nurl: https://bank.example.com',
    },
  ],
  skipped: [
    { row: 5, name: 'Wi-Fi codes', reason: 'A note entry, not a login.' },
  ],
  ignoredColumns: ['favorite', 'login_totp'],
} as const;

export function mockVaultCommands(
  state: MockState,
): Record<string, MockCommand> {
  const {
    snapshot,
    stores,
    servers,
    accounts,
    contents,
    unread,
    restoreFirstRunAccount,
  } = state;
  const describeServerStatus = mockServerStatus(state);
  // The id of the password import the mock holds, as the host holds one.
  let heldImport: string | null = null;
  const select = (request: ItemRequest) => {
    const item = state.items.find(
      (candidate) =>
        candidate.store === request.storeId && candidate.path === request.path,
    );
    // The codes the host's dirent-bound read lookup answers with.
    if (!item || item.version !== request.version)
      throw failure(
        'conflict',
        'This item changed or was removed. Refresh the vault before reading it.',
      );
    if (item.direntId !== request.direntId)
      throw failure(
        'conflict',
        'This item was replaced. Refresh the vault before reading it.',
      );
    return item;
  };
  const decodeCreateRole = (role: KvRoleInput): RoleWire => {
    if (role === 'Owner' || role === 'Admin') return { role };
    const match = /^Member:(0|-?[1-9]\d*)$/.exec(role);
    const visibility = Number(match?.[1]);
    if (
      !match ||
      !Number.isInteger(visibility) ||
      visibility < -32768 ||
      visibility > 32767
    ) {
      throw failure('invalid-request', 'Choose a valid team item role.');
    }
    return { role: 'Member', visibility };
  };
  const createRoles = (
    storeId: string,
    readRole?: KvRoleInput,
    writeRole?: KvRoleInput,
  ): { read: RoleWire; write: RoleWire } => {
    const store = stores.find((candidate) => candidate.id === storeId);
    if (!store) throw failure('store-not-found', 'This store was not found.');
    if (store.kind === 'account') {
      if (readRole !== undefined || writeRole !== undefined)
        throw failure(
          'invalid-request',
          'Account items always use the Owner role.',
        );
      return { read: { role: 'Owner' }, write: { role: 'Owner' } };
    }
    if (!store.active)
      throw failure(
        'inactive-group',
        'Finish setting up this team before making changes.',
      );
    if (readRole === undefined || writeRole === undefined)
      throw failure(
        'invalid-request',
        'Specify both read and write roles for the team item.',
      );
    return {
      read: decodeCreateRole(readRole),
      write: decodeCreateRole(writeRole),
    };
  };
  const selectBound = (
    storeId: string,
    path: string,
    version: number,
    direntId: string,
  ): Item => {
    if (unread.has(storeId))
      throw failure(
        'conflict',
        'This item changed or was removed. Refresh the vault before modifying it.',
      );
    const item = state.items.find(
      (candidate) => candidate.store === storeId && candidate.path === path,
    );
    if (!item || item.version !== version || item.direntId !== direntId)
      throw failure('conflict', `${path} changed first.`);
    return item;
  };
  const findItem = (storeId: string, path: string): Item | undefined =>
    state.items.find(
      (candidate) => candidate.store === storeId && candidate.path === path,
    );
  /** The more restrictive of two read roles, as the host orders them. */
  const stricter = (first: RoleWire, second: RoleWire): RoleWire => {
    const a = parseRole(first);
    const b = parseRole(second);
    if (!a || !b) return a ? second : first;
    const rank = roleRank(b) - roleRank(a);
    if (rank !== 0) return rank > 0 ? second : first;
    return a.kind === 'member' && visibilityOf(b) > visibilityOf(a)
      ? second
      : first;
  };
  /** The original path in a hashed entry's name, as the host hashes it. */
  const originHash = async (path: string): Promise<string> => {
    const digest = await crypto.subtle.digest(
      'SHA-256',
      new TextEncoder().encode(path),
    );
    return [...new Uint8Array(digest).slice(0, 16)]
      .map((byte) => byte.toString(16).padStart(2, '0'))
      .join('');
  };
  /** Removes an item and, for a folder, everything in it. */
  const removeTree = (storeId: string, item: Item): void => {
    const removed = (candidate: Item): boolean =>
      candidate.store === storeId &&
      (candidate === item || candidate.path.startsWith(`${item.path}/`));
    for (const candidate of state.items.filter(removed))
      contents.delete(`${storeId}|${candidate.path}`);
    state.items = state.items.filter((candidate) => !removed(candidate));
  };
  /** The `.origin` file beside a hashed Trash entry. */
  const originOf = (storeId: string, entry: Item): Item | undefined =>
    parseTrashName(entry.path.slice(entry.path.lastIndexOf('/') + 1))?.kind ===
    'hashed'
      ? findItem(storeId, `${entry.path}${ORIGIN_SUFFIX}`)
      : undefined;
  /** Moves an item, and a folder's contents with it, as MoveKv does. */
  const relocate = (storeId: string, item: Item, destination: string) => {
    const path = item.path;
    if (destination.startsWith(`${path}/`))
      throw failure('conflict', 'A folder cannot contain itself.');
    for (const candidate of state.items.filter(
      (candidate) =>
        candidate.store === storeId &&
        (candidate.path === path || candidate.path.startsWith(`${path}/`)),
    )) {
      const oldPath = candidate.path;
      candidate.path = destination + oldPath.slice(path.length);
      if (candidate === item) {
        candidate.version = 1;
        candidate.direntId = crypto.randomUUID().replaceAll('-', '');
      }
      const content = contents.get(`${storeId}|${oldPath}`);
      contents.delete(`${storeId}|${oldPath}`);
      if (content !== undefined)
        contents.set(`${storeId}|${candidate.path}`, content);
    }
  };
  const ensureFolder = (storeId: string, path: string, read: RoleWire) => {
    if (
      state.items.some((item) => item.store === storeId && item.path === path)
    )
      return;
    state.items.push({
      direntId: crypto.randomUUID().replaceAll('-', ''),
      store: storeId,
      path,
      kind: 'Folder',
      size: 0,
      version: 1,
      read,
      write: read,
    });
  };
  const assertFree = (storeId: string, path: string): void => {
    if (
      state.items.some((item) => item.store === storeId && item.path === path)
    ) {
      throw failure('already-exists', `${path} already exists.`);
    }
  };
  const catalogResponse = (): CatalogDto => {
    // Native catalog responses include directory entries with their identity and
    // version. The presentation catalog() intentionally filters those out.
    const current = { ...snapshot, stores, servers, items: state.items };
    const readable = new Set(
      stores
        .filter((store) => storeReadable(current, store.id))
        .map((store) => store.id),
    );
    return {
      profiles: [...new Set(servers.map((server) => server.profileName))],
      stores: stores.map((store) => ({ ...store })),
      knownStores: stores.map((store) => ({ ...store })),
      inventory: [...new Set(servers.map((server) => server.profileName))].map(
        (profile) => ({
          profile,
          accountsComplete: true,
          teamsComplete: true,
        }),
      ),
      items: state.items
        .filter((item) => readable.has(item.store))
        .map((item) => ({
          store: item.store,
          path: item.path,
          kind: item.kind,
          size: item.size,
          version: item.version,
          direntId: item.direntId,
          read: roleDto(item.read),
          write: roleDto(item.write),
          // The agent's listing classifies each small file from its
          // plaintext; the mock does the same from the text it serves.
          ...(item.kind === 'Secret'
            ? { shape: shapeOf(contents.get(itemKey(item)) ?? '') }
            : {}),
        })),
      failures: [],
      blockedProfiles: [],
    };
  };
  const readCatalog: MockCommand = () => {
    restoreFirstRunAccount();
    unread.clear();
    return catalogResponse();
  };
  return {
    list_catalog: readCatalog,
    list_catalog_progressive: readCatalog,
    list_profile_catalog: mockCommand(
      async ({ profile }: { profile: string }) => {
        restoreFirstRunAccount();
        const server = servers.find((server) => server.profileName === profile);
        if (!server)
          throw failure('profile-not-found', 'The profile was not found.');
        for (const store of stores)
          if (store.server === profile) unread.delete(store.id);
        const response = catalogResponse();
        const scopedStores = response.stores.filter(
          (store) => store.server === profile,
        );
        const ids = new Set(scopedStores.map((store) => store.id));
        const status = await describeServerStatus(profile).then(
          (status) => ({ status: serverStatusWire(status), error: null }),
          (error: unknown) => ({
            status: null,
            error: normalizeCommandError(error),
          }),
        );
        return {
          ...response,
          storeReads: scopedStores.map((store) => ({
            store: store.id,
            state: 'complete' as const,
          })),
          localMetadata: {
            accounts: accounts
              .filter((account) => account.server === profile)
              .map(accountWire),
            profiles: [
              {
                profile,
                label: server.displayLabel,
                configuredEndpoint: server.configuredEndpoint,
                ...status,
              },
            ],
          },
          profiles: [profile],
          fullItemReads: [profile],
          stores: scopedStores,
          knownStores: response.knownStores.filter(
            (store) => store.server === profile,
          ),
          inventory: response.inventory.filter(
            (entry) => entry.profile === profile,
          ),
          items: response.items.filter((item) => ids.has(item.store)),
        };
      },
    ),
    list_stores: () => ({ ...catalogResponse(), items: [] }),
    store_usage: mockCommand(async ({ storeId }: { storeId: string }) => {
      if (!stores.some((store) => store.id === storeId))
        throw failure('store-not-found', 'This vault is no longer available.');
      // The server counts ciphertext; the mock counts the plaintext it holds.
      const files = state.items.filter(
        (item) => item.store === storeId && item.kind !== 'Folder',
      );
      return {
        files: files.length,
        bytes: files.reduce(
          (total, item) =>
            total +
            (item.size ??
              new TextEncoder().encode(contents.get(itemKey(item)) ?? '')
                .length),
          0,
        ),
      };
    }),
    read_item: mockCommand((request: ItemRequest) => {
      const item = select(request);
      return Promise.resolve({
        store: item.store,
        path: item.path,
        version: item.version,
        value: contents.get(itemKey(item)) ?? '',
      });
    }),
    copy_item_value: mockCommand((request: ItemRequest) => {
      const item = select(request);
      mockCopies.push({
        store: item.store,
        path: item.path,
        field: 'value',
        text: contents.get(itemKey(item)) ?? '',
      });
      return Promise.resolve(MOCK_CLIPBOARD_COPY);
    }),
    copy_item_field: mockCommand(
      async ({ field, ...request }: CopyFieldRequest) => {
        const item = select(request);
        const stored = contents.get(itemKey(item)) ?? '';
        const text = field === 'value' ? stored : loginField(stored, field);
        if (text === null)
          throw failure(
            'field-missing',
            `This item has no ${FIELD_NAMES[field as keyof typeof FIELD_NAMES]}.`,
          );
        mockCopies.push({ store: item.store, path: item.path, field, text });
        return MOCK_CLIPBOARD_COPY;
      },
    ),
    open_item_url: mockCommand(async (request: ItemRequest) => {
      const item = select(request);
      const field = loginField(contents.get(itemKey(item)) ?? '', 'url');
      if (field === null)
        throw failure('field-missing', `This item has no ${FIELD_NAMES.url}.`);
      const url = mockWebsiteUrl(field);
      if (url === null)
        throw failure(
          'website-unsupported',
          "This item's website is not an http or https address.",
        );
      mockOpenedSites.push({ store: item.store, path: item.path, url });
      return { ok: true };
    }),
    download_file: mockCommand((request: ItemRequest) => {
      select(request);
      return Promise.resolve({ saved: true });
    }),
    create_text_item: mockCommand(
      async ({
        storeId,
        path,
        value,
        readRole,
        writeRole,
      }: CreateTextRequest) => {
        assertFree(storeId, path);
        const roles = createRoles(storeId, readRole, writeRole);
        state.items.push({
          direntId: crypto.randomUUID().replaceAll('-', ''),
          store: storeId,
          path,
          kind: 'Secret',
          size: value.length,
          version: 1,
          ...roles,
        });
        contents.set(`${storeId}|${path}`, value);
        return { applied: true };
      },
    ),
    create_folder: mockCommand(
      async ({ storeId, path, readRole, writeRole }: CreateFolderRequest) => {
        assertFree(storeId, path);
        const roles = createRoles(storeId, readRole, writeRole);
        state.items.push({
          direntId: crypto.randomUUID().replaceAll('-', ''),
          store: storeId,
          path,
          kind: 'Folder',
          size: 0,
          version: 1,
          ...roles,
        });
        return { applied: true };
      },
    ),
    edit_text_item: mockCommand(
      async ({ storeId, path, version, direntId, value }: EditTextRequest) => {
        const index = state.items.findIndex(
          (item) => item.store === storeId && item.path === path,
        );
        const item = index < 0 ? undefined : state.items[index];
        if (!item)
          throw failure('conflict', `Item '${path}' was deleted or moved.`);
        if (item.version !== version || item.direntId !== direntId)
          throw failure('conflict', `${path} changed first.`);
        state.items[index] = {
          ...item,
          version: version + 1,
          size: value.length,
        };
        contents.set(`${storeId}|${path}`, value);
        return { applied: true };
      },
    ),
    move_item: mockCommand(
      async ({
        storeId,
        path,
        version,
        destination,
        direntId,
      }: MoveItemRequest) => {
        const item = selectBound(storeId, path, version, direntId);
        assertFree(storeId, destination);
        relocate(storeId, item, destination);
        unread.add(storeId);
        return { applied: true };
      },
    ),
    copy_item_to_store: mockCommand(
      async ({
        storeId,
        path,
        version,
        direntId,
        targetStoreId,
        targetPath,
        readRole,
        writeRole,
      }: CopyItemToStoreRequest) => {
        const item = selectBound(storeId, path, version, direntId);
        if (targetStoreId === storeId)
          throw failure(
            'invalid-request',
            'Choose another vault. Rename or move changes where an item is within its vault.',
          );
        if (item.kind === 'Folder')
          throw failure(
            'invalid-request',
            'Folders cannot be copied to another vault. Copy the items in it one at a time.',
          );
        const roles = createRoles(targetStoreId, readRole, writeRole);
        assertFree(targetStoreId, targetPath);
        state.items.push({
          ...item,
          direntId: crypto.randomUUID().replaceAll('-', ''),
          store: targetStoreId,
          path: targetPath,
          version: 1,
          ...roles,
        });
        const content = contents.get(`${storeId}|${path}`);
        if (content !== undefined)
          contents.set(`${targetStoreId}|${targetPath}`, content);
        unread.add(targetStoreId);
        return { applied: true };
      },
    ),
    trash_item: mockCommand(
      async ({ storeId, path, version, direntId }: RemoveItemRequest) => {
        const item = selectBound(storeId, path, version, direntId);
        if (isTrashPath(path))
          throw failure('invalid-request', 'This item is already in Trash.');
        // Filed by the most restrictive read role on the path, as the host does.
        let read = item.read;
        for (
          let cut = path.indexOf('/', 1);
          cut > 0;
          cut = path.indexOf('/', cut + 1)
        ) {
          const ancestor = findItem(storeId, path.slice(0, cut));
          if (ancestor?.kind === 'Folder') read = stricter(read, ancestor.read);
        }
        const role = parseRole(read);
        const tag =
          role?.kind === 'member'
            ? `member-${visibilityOf(role)}`
            : (role?.kind ?? 'owner');
        const folder = `${TRASH_ROOT}/${tag}`;
        const team =
          stores.find((store) => store.id === storeId)?.kind === 'team';
        ensureFolder(
          storeId,
          TRASH_ROOT,
          team ? 'Member · visibility -16384' : 'Owner',
        );
        ensureFolder(storeId, folder, read);
        const stamp = trashStamp(new Date());
        let name = `${stamp}~${encodeOriginalPath(path)}`;
        const hashed = new TextEncoder().encode(name).length > 255;
        if (hashed) name = `${stamp}~#${await originHash(path)}`;
        const destination = `${folder}/${name}`;
        if (findItem(storeId, destination))
          throw failure(
            'trash-name-taken',
            'An entry with the same name is already in Trash. Try again in a second.',
          );
        if (hashed) {
          const origin = `${destination}${ORIGIN_SUFFIX}`;
          state.items.push({
            direntId: crypto.randomUUID().replaceAll('-', ''),
            store: storeId,
            path: origin,
            kind: 'Secret',
            size: path.length,
            version: 1,
            read,
            write: item.write,
          });
          contents.set(`${storeId}|${origin}`, path);
        }
        relocate(storeId, item, destination);
        unread.add(storeId);
        return { applied: true, path: destination };
      },
    ),
    restore_trash_item: mockCommand(
      async ({
        storeId,
        path,
        version,
        direntId,
        destination,
      }: RestoreTrashRequest) => {
        const item = selectBound(storeId, path, version, direntId);
        if (!isTrashPath(path) || isTrashPath(destination))
          throw failure(
            'invalid-request',
            'Only entries in Trash can be restored, to a place outside Trash.',
          );
        assertFree(storeId, destination);
        const origin = originOf(storeId, item);
        relocate(storeId, item, destination);
        if (origin) removeTree(storeId, origin);
        unread.add(storeId);
        return { applied: true, path: destination };
      },
    ),
    remove_item: mockCommand(
      async ({
        storeId,
        path,
        version,
        direntId,
        outsideTrash,
      }: PermanentRemoveRequest) => {
        const item = selectBound(storeId, path, version, direntId);
        if (outsideTrash) {
          const team =
            stores.find((store) => store.id === storeId)?.kind === 'team';
          if (
            !team ||
            isTrashPath(path) ||
            findItem(storeId, TRASH_ROOT) ||
            item.kind === 'Folder'
          )
            throw failure(
              'invalid-request',
              'Move this item to Trash; only items in Trash can be deleted permanently.',
            );
        } else if (!path.startsWith(`${TRASH_ROOT}/`))
          throw failure(
            'invalid-request',
            'Only items in Trash can be deleted permanently. Move this item to Trash first.',
          );
        const origin = originOf(storeId, item);
        removeTree(storeId, item);
        if (origin) removeTree(storeId, origin);
        unread.add(storeId);
        return { applied: true };
      },
    ),
    empty_trash: mockCommand(
      async ({ storeId, entries }: EmptyTrashRequest) => {
        if (!entries.length)
          throw failure(
            'invalid-request',
            'Choose at least one entry in Trash.',
          );
        const selected = entries.map((entry) =>
          selectBound(storeId, entry.path, entry.version, entry.direntId),
        );
        if (selected.some((item) => !item.path.startsWith(`${TRASH_ROOT}/`)))
          throw failure(
            'invalid-request',
            'Only items in Trash can be deleted permanently. Move this item to Trash first.',
          );
        for (const item of selected) {
          const origin = originOf(storeId, item);
          removeTree(storeId, item);
          if (origin) removeTree(storeId, origin);
        }
        // `.origin` files left without their entry go too.
        for (const item of state.items.filter(
          (candidate) =>
            candidate.store === storeId &&
            parseTrashName(
              candidate.path.slice(candidate.path.lastIndexOf('/') + 1),
            )?.kind === 'origin' &&
            !findItem(storeId, candidate.path.slice(0, -ORIGIN_SUFFIX.length)),
        ))
          removeTree(storeId, item);
        unread.add(storeId);
        return { applied: true };
      },
    ),
    // The mock records no durable uploads. The native command passes the
    // agent's answer through, so these answer in its snake_case form.
    list_file_uploads: mockCommand(async () => ({
      uploads: [],
      next: null,
      capacity: {
        max_reserved_bytes: 268435456,
        max_pending: 16,
        max_receipts: 8192,
        max_receipt_bytes: 16777216,
      },
    })),
    resume_file_upload: mockCommand(async () => {
      throw failure('not-recorded', 'Upload is not recorded.');
    }),
    cancel_file_upload: mockCommand(async () => {
      throw failure('not-recorded', 'Upload is not recorded.');
    }),
    import_dropped_file: mockCommand(
      async ({
        storeId,
        path,
        readRole,
        writeRole,
      }: ImportDroppedFileRequest) => {
        assertFree(storeId, path);
        const roles = createRoles(storeId, readRole, writeRole);
        state.items.push({
          direntId: crypto.randomUUID().replaceAll('-', ''),
          store: storeId,
          path,
          kind: 'File',
          size: 0,
          version: 1,
          ...roles,
        });
        return { applied: true };
      },
    ),
    pick_import_file: () => '/tmp/picked-file',
    begin_password_import: mockCommand(async () => {
      heldImport = crypto.randomUUID().replaceAll('-', '');
      return {
        id: heldImport,
        fileName: MOCK_PASSWORD_IMPORT.fileName,
        entries: MOCK_PASSWORD_IMPORT.entries.map(({ name, folder }) => ({
          name,
          folder,
          document: false,
        })),
        skipped: MOCK_PASSWORD_IMPORT.skipped.map((skip) => ({ ...skip })),
        ignoredColumns: [...MOCK_PASSWORD_IMPORT.ignoredColumns],
      };
    }),
    commit_password_import: mockCommand(
      async ({
        importId,
        entries,
        storeId,
        folder,
        readRole,
        writeRole,
        onProgress,
      }: CommitPasswordImportRequest & { onProgress: TransportChannel }) => {
        if (importId !== heldImport)
          throw failure(
            'import-expired',
            'This import is no longer held. Choose the file again.',
          );
        const roles = createRoles(storeId, readRole, writeRole);
        return entries.map((index) => {
          const entry = MOCK_PASSWORD_IMPORT.entries[index];
          if (!entry)
            throw failure(
              'invalid-request',
              'Choose logins this import holds.',
            );
          const path = [folder === '/' ? '' : folder, entry.folder, entry.name]
            .filter((part, at) => at === 0 || part)
            .join('/');
          let outcome: {
            index: number;
            status: 'created' | 'exists';
            message?: string;
          };
          if (
            state.items.some(
              (item) => item.store === storeId && item.path === path,
            )
          )
            outcome = {
              index,
              status: 'exists',
              message: 'Something with this name is already in the vault.',
            };
          else {
            state.items.push({
              direntId: crypto.randomUUID().replaceAll('-', ''),
              store: storeId,
              path,
              kind: 'Secret',
              size: entry.text.length,
              version: 1,
              ...roles,
            });
            contents.set(`${storeId}|${path}`, entry.text);
            unread.add(storeId);
            outcome = { index, status: 'created' };
          }
          onProgress.onmessage(outcome);
          return outcome;
        });
      },
    ),
    export_logins: mockCommand(async ({ storeId }: { storeId: string }) => {
      // The mock saves nowhere; it counts what the host would write.
      const exported = state.items.filter(
        (item) =>
          item.store === storeId &&
          item.kind === 'Secret' &&
          !isTrashPath(item.path) &&
          shapeOf(contents.get(itemKey(item)) ?? '') === 'login',
      ).length;
      if (!exported)
        throw failure(
          'invalid-request',
          'This vault has no passwords to export.',
        );
      return { saved: true, exported, skipped: 0 };
    }),
    discard_password_import: mockCommand(
      async ({ importId }: { importId: string }) => {
        if (importId === heldImport) heldImport = null;
        return { ok: true };
      },
    ),
    release_import_file: () => ({ ok: true }),
    replace_dropped_file: mockCommand(
      async ({
        storeId,
        path,
        version,
        direntId,
      }: ReplaceDroppedFileRequest) => {
        const item = state.items.find(
          (candidate) => candidate.store === storeId && candidate.path === path,
        );
        if (!item || item.version !== version || item.direntId !== direntId)
          throw failure('conflict', `${path} changed first.`);
        item.version += 1;
        return { applied: true };
      },
    ),
    pick_and_replace_file: mockCommand(
      async ({ storeId, path, version, direntId }: RemoveItemRequest) => {
        const item = state.items.find(
          (candidate) => candidate.store === storeId && candidate.path === path,
        );
        if (!item || item.version !== version || item.direntId !== direntId)
          throw failure('conflict', `${path} changed first.`);
        item.version += 1;
        return { applied: true };
      },
    ),
  };
}
