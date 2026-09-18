import { Channel } from '@tauri-apps/api/core';
import type { Bridge } from './contract';
import { checked, checkedMutation } from './transport';
import {
  decodeCatalog,
  decodeCopy,
  decodeDownload,
  decodeMutation,
  decodeReadItem,
} from './vault-catalog';

export const vaultCommands: Pick<
  Bridge,
  | 'listCatalog'
  | 'listProfileCatalog'
  | 'listStores'
  | 'readItem'
  | 'copyItemValue'
  | 'copyItemPath'
  | 'downloadFile'
  | 'createTextItem'
  | 'createLink'
  | 'createFolder'
  | 'editTextItem'
  | 'removeItem'
  | 'importDroppedFile'
  | 'pickAndImportFile'
  | 'replaceDroppedFile'
  | 'pickAndReplaceFile'
> = {
  listCatalog: async (onPartial) => {
    if (!onPartial) return checked('list_catalog', undefined, decodeCatalog);
    const channel = new Channel<unknown>();
    let failure: unknown;
    channel.onmessage = (value) => {
      try {
        onPartial(decodeCatalog(value));
      } catch (error) {
        failure = error;
      }
    };
    try {
      const catalog = await checked(
        'list_catalog_progressive',
        { onPartial: channel },
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
  editTextItem: ({ storeId, path, version, value }) =>
    checkedMutation(
      'edit_text_item',
      { storeId, path, version, value },
      decodeMutation,
    ),
  removeItem: ({ storeId, path, version }) =>
    checkedMutation('remove_item', { storeId, path, version }, decodeMutation),
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
  pickAndImportFile: ({ storeId, path, readRole, writeRole }) =>
    checkedMutation(
      'pick_and_import_file',
      {
        storeId,
        path,
        ...(readRole === undefined ? {} : { readRole }),
        ...(writeRole === undefined ? {} : { writeRole }),
      },
      decodeMutation,
    ),
  replaceDroppedFile: ({ storeId, path, version, sourcePath }) =>
    checkedMutation(
      'replace_dropped_file',
      { storeId, path, version, sourcePath },
      decodeMutation,
    ),
  pickAndReplaceFile: ({ storeId, path, version }) =>
    checkedMutation(
      'pick_and_replace_file',
      { storeId, path, version },
      decodeMutation,
    ),
};
