import type { Bridge } from './contract';
import {
  isTerminalCommandError,
  normalizeCommandError,
  type CommandError,
} from './errors';
import type { Transport } from './transport';
import { decodeCommandAck } from './core';
import { array, nullableString } from './validation';
import {
  decodeCatalog,
  decodeClipboardCopy,
  decodeCopy,
  decodeDownload,
  decodeLoginExport,
  decodeMutation,
  decodePasswordImportOutcome,
  decodePasswordImportPreview,
  decodeReadItem,
  decodeStoreUsage,
  decodeTrashMutation,
} from './vault-catalog';

export const vaultCommands = (
  transport: Transport,
): Pick<
  Bridge,
  | 'listCatalog'
  | 'listProfileCatalog'
  | 'listStores'
  | 'readItem'
  | 'storeUsage'
  | 'copyItemValue'
  | 'copyItemField'
  | 'openItemUrl'
  | 'downloadFile'
  | 'createTextItem'
  | 'createFolder'
  | 'editTextItem'
  | 'moveItem'
  | 'copyItemToStore'
  | 'trashItem'
  | 'restoreTrashItem'
  | 'removeItem'
  | 'emptyTrash'
  | 'importDroppedFile'
  | 'pickImportFile'
  | 'releaseImportFile'
  | 'replaceDroppedFile'
  | 'pickAndReplaceFile'
  | 'beginPasswordImport'
  | 'commitPasswordImport'
  | 'discardPasswordImport'
  | 'exportLogins'
> => ({
  listCatalog: async (onPartial, fresh = false) => {
    if (!onPartial)
      return transport.checked('list_catalog', { fresh }, decodeCatalog);
    const channel = transport.channel();
    let failure: CommandError | undefined;
    channel.onmessage = (value) => {
      if (failure) return;
      try {
        onPartial(decodeCatalog(value));
      } catch (cause) {
        const error = normalizeCommandError(cause);
        if (isTerminalCommandError(error)) failure = error;
      }
    };
    try {
      const catalog = await transport.checked(
        'list_catalog_progressive',
        { onPartial: channel, fresh },
        decodeCatalog,
      );
      if (failure) throw failure;
      return catalog;
    } finally {
      channel.onmessage = () => undefined;
    }
  },
  listProfileCatalog: (profile) =>
    transport.checked('list_profile_catalog', { profile }, decodeCatalog),
  listStores: () => transport.checked('list_stores', undefined, decodeCatalog),
  readItem: ({ storeId, path, version }) =>
    transport.checked('read_item', { storeId, path, version }, decodeReadItem),
  storeUsage: (storeId) =>
    transport.checked('store_usage', { storeId }, decodeStoreUsage),
  copyItemValue: ({ storeId, path, version }) =>
    transport.checked(
      'copy_item_value',
      { storeId, path, version },
      decodeClipboardCopy,
    ),
  copyItemField: ({ storeId, path, version, field }) =>
    transport.checked(
      'copy_item_field',
      { storeId, path, version, field },
      decodeClipboardCopy,
    ),
  openItemUrl: ({ storeId, path, version }) =>
    transport.checked('open_item_url', { storeId, path, version }, decodeCopy),
  downloadFile: ({ storeId, path, version }) =>
    transport.checked(
      'download_file',
      { storeId, path, version },
      decodeDownload,
    ),
  createTextItem: ({ storeId, path, value, readRole, writeRole }) =>
    transport.checkedMutation(
      'create_text_item',
      {
        storeId,
        path,
        value,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  createFolder: ({ storeId, path, readRole, writeRole }) =>
    transport.checkedMutation(
      'create_folder',
      {
        storeId,
        path,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  editTextItem: ({ storeId, path, version, direntId, value }) =>
    transport.checkedMutation(
      'edit_text_item',
      { storeId, path, version, direntId, value },
      decodeMutation,
    ),
  moveItem: ({ storeId, path, version, destination, direntId }) =>
    transport.checkedMutation(
      'move_item',
      { storeId, path, version, destination, direntId },
      decodeMutation,
    ),
  copyItemToStore: ({
    storeId,
    path,
    version,
    direntId,
    targetStoreId,
    targetPath,
    readRole,
    writeRole,
  }) =>
    transport.checkedMutation(
      'copy_item_to_store',
      {
        storeId,
        path,
        version,
        direntId,
        targetStoreId,
        targetPath,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  trashItem: ({ storeId, path, version, direntId }) =>
    transport.checkedMutation(
      'trash_item',
      { storeId, path, version, direntId },
      decodeTrashMutation,
    ),
  restoreTrashItem: ({ storeId, path, version, direntId, destination }) =>
    transport.checkedMutation(
      'restore_trash_item',
      { storeId, path, version, direntId, destination },
      decodeTrashMutation,
    ),
  removeItem: ({ storeId, path, version, direntId, outsideTrash }) =>
    transport.checkedMutation(
      'remove_item',
      {
        storeId,
        path,
        version,
        direntId,
        outsideTrash: outsideTrash === true,
      },
      decodeMutation,
    ),
  emptyTrash: ({ storeId, entries }) =>
    transport.checkedMutation(
      'empty_trash',
      {
        storeId,
        entries: entries.map(({ path, version, direntId }) => ({
          path,
          version,
          direntId,
        })),
      },
      decodeMutation,
    ),
  importDroppedFile: ({ storeId, path, sourcePath, readRole, writeRole }) =>
    transport.checkedMutation(
      'import_dropped_file',
      {
        storeId,
        path,
        sourcePath,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  pickImportFile: () =>
    transport.checked('pick_import_file', undefined, (value) =>
      nullableString(value, 'pick_import_file response'),
    ),
  releaseImportFile: (sourcePath) =>
    transport.checked(
      'release_import_file',
      { sourcePath },
      decodeCommandAck,
      false,
    ),
  replaceDroppedFile: ({ storeId, path, version, direntId, sourcePath }) =>
    transport.checkedMutation(
      'replace_dropped_file',
      { storeId, path, version, direntId, sourcePath },
      decodeMutation,
    ),
  pickAndReplaceFile: ({ storeId, path, version, direntId }) =>
    transport.checkedMutation(
      'pick_and_replace_file',
      { storeId, path, version, direntId },
      decodeMutation,
    ),
  beginPasswordImport: () =>
    transport.checked('begin_password_import', undefined, (value) =>
      value === null ? null : decodePasswordImportPreview(value),
    ),
  commitPasswordImport: async (
    { importId, entries, storeId, folder, readRole, writeRole },
    onProgress,
  ) => {
    const channel = transport.channel();
    channel.onmessage = (value) => {
      // The resolved list repeats every outcome, so a progress message
      // that fails to decode is only a missed update.
      try {
        onProgress?.(decodePasswordImportOutcome(value));
      } catch {
        // Ignored; see above.
      }
    };
    try {
      return await transport.checkedMutation(
        'commit_password_import',
        {
          importId,
          entries: [...entries],
          storeId,
          folder,
          ...(readRole === undefined ? {} : { readRole }),
          ...(writeRole === undefined ? {} : { writeRole }),
          onProgress: channel,
        },
        (value) =>
          array(value, 'commit_password_import response', (entry, at) =>
            decodePasswordImportOutcome(entry, at),
          ),
      );
    } finally {
      channel.onmessage = () => undefined;
    }
  },
  discardPasswordImport: (importId) =>
    transport.checked(
      'discard_password_import',
      { importId },
      decodeCommandAck,
      false,
    ),
  exportLogins: (storeId) =>
    transport.checked('export_logins', { storeId }, decodeLoginExport),
});
