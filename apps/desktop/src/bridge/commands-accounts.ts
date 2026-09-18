import { decodeBotReply } from '../bot-contract';
import { decodeRenameProgress } from '../rename-contract';
import { decodeSsoProgress } from '../sso-contract';
import {
  decodeAccounts,
  decodeFederation,
  decodeGroupDetails,
  decodeGroupDiscovery,
  decodeParties,
} from './accounts-groups';
import type { Bridge } from './contract';
import { decodeCommandAck } from './core';
import { checked } from './transport';
import { record } from './validation';
import { decodeCopy, decodeMutation } from './vault-catalog';

export const accountCommands: Pick<
  Bridge,
  | 'listAccounts'
  | 'listGroupDetails'
  | 'listParties'
  | 'listFederation'
  | 'resumeGroupCreation'
  | 'abandonGroupCreation'
  | 'createGroup'
  | 'addGroupMember'
  | 'resumeGroupMemberAddition'
  | 'demoteGroupMember'
  | 'removeGroupMember'
  | 'resumeGroupMemberEdit'
  | 'admitGroup'
  | 'removeFederatedGroup'
  | 'rerunGroupAdmission'
  | 'discoverGroups'
  | 'configureWebAdmin'
  | 'openWebAdmin'
  | 'botAccount'
  | 'setLocalAccountAlias'
  | 'renameAccount'
  | 'sso'
  | 'openSsoBrowser'
> = {
  listAccounts: (generation) =>
    checked(
      'list_accounts',
      generation === undefined ? undefined : { generation },
      decodeAccounts,
    ),
  listGroupDetails: (storeId) =>
    checked('list_group_details', { storeId }, decodeGroupDetails),
  listParties: (storeId) => checked('list_parties', { storeId }, decodeParties),
  listFederation: (storeId) =>
    checked('list_federation', { storeId }, decodeFederation),
  resumeGroupCreation: (storeId) =>
    checked('resume_group_creation', { storeId }, decodeMutation),
  abandonGroupCreation: (storeId) =>
    checked('abandon_group_creation', { storeId }, decodeMutation),
  createGroup: ({ accountStoreId, teamAlias, name, kind }) =>
    checked(
      'create_group',
      { accountStoreId, teamAlias, name, kind },
      decodeMutation,
    ),
  addGroupMember: ({ storeId, username, destination }) =>
    checked(
      'add_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  resumeGroupMemberAddition: ({ storeId, username }) =>
    checked(
      'resume_group_member_addition',
      { storeId, username },
      decodeMutation,
    ),
  demoteGroupMember: ({ storeId, username, destination }) =>
    checked(
      'demote_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  removeGroupMember: ({ storeId, username }) =>
    checked('remove_group_member', { storeId, username }, decodeMutation),
  resumeGroupMemberEdit: (storeId) =>
    checked('resume_group_member_edit', { storeId }, decodeMutation),
  admitGroup: ({ storeId, remoteStoreId, visibility }) =>
    checked(
      'admit_group',
      { storeId, remoteStoreId, visibility },
      decodeMutation,
    ),
  removeFederatedGroup: ({ storeId, remoteHostIdHex, remoteTeamIdHex }) =>
    checked(
      'expel_federated_group',
      { storeId, remoteHostIdHex, remoteTeamIdHex },
      decodeMutation,
    ),
  rerunGroupAdmission: (storeId, operationId) =>
    checked('rerun_group_admission', { storeId, operationId }, decodeMutation),
  configureWebAdmin: (profile, accountAlias, destination) =>
    checked(
      'configure_web_admin',
      { profile, accountAlias, destination },
      decodeCommandAck,
    ),
  openWebAdmin: (profile, accountAlias, pin) =>
    checked('open_web_admin', { profile, accountAlias, pin }, decodeCommandAck),
  botAccount: (profile, accountAlias, action) =>
    checked(
      'bot_account_request',
      { profile, accountAlias, action },
      decodeBotReply,
    ),
  setLocalAccountAlias: (store, label) =>
    checked(
      'set_local_account_alias',
      { accountStoreId: store, label },
      (value) => {
        const item = record(value, 'local alias response');
        if (
          Object.keys(item).sort().join(',') !== 'alias,store' ||
          item.store !== store ||
          item.alias !== label
        )
          throw new Error(
            'Local alias response belongs to a different account or label.',
          );
        return { store, alias: label };
      },
    ),
  renameAccount: (profile, accountAlias, action) =>
    checked(
      'rename_account_request',
      { profile, accountAlias, action },
      decodeRenameProgress,
    ),
  sso: (profile, accountAlias, action) =>
    checked(
      'sso_request',
      { profile, accountAlias, action },
      decodeSsoProgress,
    ),
  openSsoBrowser: (profile, accountAlias, operationId) =>
    checked(
      'open_sso_browser',
      { profile, accountAlias, operationId },
      decodeCopy,
    ),
  discoverGroups: (profile, accountAlias) =>
    checked('discover_groups', { profile, accountAlias }, decodeGroupDiscovery),
};
