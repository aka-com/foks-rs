import { Channel } from '@tauri-apps/api/core';
import type { Bridge } from './contract';
import {
  isTerminalCommandError,
  normalizeCommandError,
  type CommandError,
} from './errors';
import { checked, checkedMutation } from './transport';
import { decodeCommandAck } from './core';
import { array, nullableString } from './validation';
import {
  decodeCatalog,
  decodeCopy,
  decodeDownload,
  decodeMutation,
  decodePasswordImportOutcome,
  decodePasswordImportPreview,
  decodeReadItem,
  decodeTrashMutation,
} from './vault-catalog';

export const vaultCommands: Pick<
  Bridge,
  | 'listCatalog'
  | 'listProfileCatalog'
  | 'listStores'
  | 'readItem'
  | 'copyItemValue'
  | 'copyItemField'
  | 'copyItemPath'
  | 'downloadFile'
  | 'createTextItem'
  | 'createLink'
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
> = {
  listCatalog: async (onPartial, fresh = false) => {
    if (!onPartial) return checked('list_catalog', { fresh }, decodeCatalog);
    const channel = new Channel<unknown>();
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
      const catalog = await checked(
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
    checked('list_profile_catalog', { profile }, decodeCatalog),
  listStores: () => checked('list_stores', undefined, decodeCatalog),
  readItem: ({ storeId, path, version }) =>
    checked('read_item', { storeId, path, version }, decodeReadItem),
  copyItemValue: ({ storeId, path, version }) =>
    checked('copy_item_value', { storeId, path, version }, decodeCopy),
  copyItemField: ({ storeId, path, version, field }) =>
    checked('copy_item_field', { storeId, path, version, field }, decodeCopy),
  copyItemPath: ({ storeId, path, version }) =>
    checked('copy_item_path', { storeId, path, version }, decodeCopy),
  downloadFile: ({ storeId, path, version }) =>
    checked('download_file', { storeId, path, version }, decodeDownload),
  createTextItem: ({ storeId, path, value, readRole, writeRole }) =>
    checkedMutation(
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
  createLink: ({ storeId, path, target, readRole, writeRole }) =>
    checkedMutation(
      'create_link',
      {
        storeId,
        path,
        target,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  createFolder: ({ storeId, path, readRole, writeRole }) =>
    checkedMutation(
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
    checkedMutation(
      'edit_text_item',
      { storeId, path, version, direntId, value },
      decodeMutation,
    ),
  moveItem: ({ storeId, path, version, destination, direntId }) =>
    checkedMutation(
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
    checkedMutation(
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
    checkedMutation(
      'trash_item',
      { storeId, path, version, direntId },
      decodeTrashMutation,
    ),
  restoreTrashItem: ({ storeId, path, version, direntId, destination }) =>
    checkedMutation(
      'restore_trash_item',
      { storeId, path, version, direntId, destination },
      decodeTrashMutation,
    ),
  removeItem: ({ storeId, path, version, direntId, outsideTrash }) =>
    checkedMutation(
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
    checkedMutation(
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
    checkedMutation(
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
    checked('pick_import_file', undefined, (value) =>
      nullableString(value, 'pick_import_file response'),
    ),
  releaseImportFile: (sourcePath) =>
    checked('release_import_file', { sourcePath }, decodeCommandAck, false),
  replaceDroppedFile: ({ storeId, path, version, direntId, sourcePath }) =>
    checkedMutation(
      'replace_dropped_file',
      { storeId, path, version, direntId, sourcePath },
      decodeMutation,
    ),
  pickAndReplaceFile: ({ storeId, path, version, direntId }) =>
    checkedMutation(
      'pick_and_replace_file',
      { storeId, path, version, direntId },
      decodeMutation,
    ),
  beginPasswordImport: () =>
    checked('begin_password_import', undefined, (value) =>
      value === null ? null : decodePasswordImportPreview(value),
    ),
  commitPasswordImport: async (
    { importId, entries, storeId, folder, readRole, writeRole },
    onProgress,
  ) => {
    const channel = new Channel<unknown>();
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
      return await checkedMutation(
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
    checked('discard_password_import', { importId }, decodeCommandAck, false),
};
