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
import { checked, checkedMutation } from './transport';
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
    checkedMutation('resume_group_creation', { storeId }, decodeMutation),
  abandonGroupCreation: (storeId) =>
    checkedMutation('abandon_group_creation', { storeId }, decodeMutation),
  createGroup: ({ accountStoreId, teamAlias, name, kind }) =>
    checkedMutation(
      'create_group',
      { accountStoreId, teamAlias, name, kind },
      decodeMutation,
    ),
  addGroupMember: ({ storeId, username, destination }) =>
    checkedMutation(
      'add_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  resumeGroupMemberAddition: ({ storeId, username }) =>
    checkedMutation(
      'resume_group_member_addition',
      { storeId, username },
      decodeMutation,
    ),
  demoteGroupMember: ({ storeId, username, destination }) =>
    checkedMutation(
      'demote_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  removeGroupMember: ({ storeId, username }) =>
    checkedMutation(
      'remove_group_member',
      { storeId, username },
      decodeMutation,
    ),
  resumeGroupMemberEdit: (storeId) =>
    checkedMutation('resume_group_member_edit', { storeId }, decodeMutation),
  admitGroup: ({ storeId, remoteStoreId, visibility }) =>
    checkedMutation(
      'admit_group',
      { storeId, remoteStoreId, visibility },
      decodeMutation,
    ),
  removeFederatedGroup: ({ storeId, remoteHostIdHex, remoteTeamIdHex }) =>
    checkedMutation(
      'expel_federated_group',
      { storeId, remoteHostIdHex, remoteTeamIdHex },
      decodeMutation,
    ),
  rerunGroupAdmission: (storeId, operationId) =>
    checkedMutation(
      'rerun_group_admission',
      { storeId, operationId },
      decodeMutation,
    ),
  configureWebAdmin: (profile, accountAlias, destination) =>
    checkedMutation(
      'configure_web_admin',
      { profile, accountAlias, destination },
      decodeCommandAck,
    ),
  openWebAdmin: (profile, accountAlias, pin) =>
    checkedMutation(
      'open_web_admin',
      { profile, accountAlias, pin },
      decodeCommandAck,
    ),
  botAccount: (profile, accountAlias, action) =>
    (['list', 'status'].includes(action.action) ? checked : checkedMutation)(
      'bot_account_request',
      { profile, accountAlias, action },
      decodeBotReply,
    ),
  setLocalAccountAlias: (store, label) =>
    checkedMutation(
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
    (action === null || action.action === 'status' ? checked : checkedMutation)(
      'rename_account_request',
      { profile, accountAlias, action },
      decodeRenameProgress,
    ),
  sso: (profile, accountAlias, action) =>
    (['account-status', 'status', 'poll'].includes(action.action)
      ? checked
      : checkedMutation)(
      'sso_request',
      { profile, accountAlias, action },
      decodeSsoProgress,
    ),
  openSsoBrowser: (profile, accountAlias, operationId) =>
    checkedMutation(
      'open_sso_browser',
      { profile, accountAlias, operationId },
      decodeCopy,
    ),
  discoverGroups: (profile, accountAlias) =>
    checked('discover_groups', { profile, accountAlias }, decodeGroupDiscovery),
};
