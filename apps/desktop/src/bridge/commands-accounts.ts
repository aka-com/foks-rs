import { decodeAdapterClock } from '../adapter-clock-contract';
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
import type { Transport } from './transport';
import { record } from './validation';
import { decodeCopy, decodeMutation } from './vault-catalog';

export const accountCommands = (
  transport: Transport,
): Pick<
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
  | 'promoteGroupMember'
  | 'demoteGroupMember'
  | 'removeGroupMember'
  | 'resumeGroupMemberEdit'
  | 'addFederatedTeamMember'
  | 'removeFederatedGroup'
  | 'rerunFederatedTeamMemberAdd'
  | 'discoverGroups'
  | 'configureWebAdmin'
  | 'openWebAdmin'
  | 'botAccount'
  | 'setLocalAccountAlias'
  | 'renameAccount'
  | 'adapterClock'
  | 'sso'
  | 'openSsoBrowser'
> => ({
  listAccounts: (generation) =>
    transport.checked(
      'list_accounts',
      generation === undefined ? undefined : { generation },
      decodeAccounts,
    ),
  listGroupDetails: (storeId) =>
    transport.checked('list_group_details', { storeId }, decodeGroupDetails),
  listParties: (storeId) =>
    transport.checked('list_parties', { storeId }, decodeParties),
  listFederation: (storeId) =>
    transport.checked('list_federation', { storeId }, decodeFederation),
  resumeGroupCreation: (storeId) =>
    transport.checkedMutation(
      'resume_group_creation',
      { storeId },
      decodeMutation,
    ),
  abandonGroupCreation: (storeId) =>
    transport.checkedMutation(
      'abandon_group_creation',
      { storeId },
      decodeMutation,
    ),
  createGroup: ({ accountStoreId, teamAlias, name, kind }) =>
    transport.checkedMutation(
      'create_group',
      { accountStoreId, teamAlias, name, kind },
      decodeMutation,
    ),
  addGroupMember: ({ storeId, username, destination }) =>
    transport.checkedMutation(
      'add_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  resumeGroupMemberAddition: ({ storeId, username }) =>
    transport.checkedMutation(
      'resume_group_member_addition',
      { storeId, username },
      decodeMutation,
    ),
  promoteGroupMember: ({ storeId, username, destination }) =>
    transport.checkedMutation(
      'promote_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  demoteGroupMember: ({ storeId, username, destination }) =>
    transport.checkedMutation(
      'demote_group_member',
      { storeId, username, destination },
      decodeMutation,
    ),
  removeGroupMember: ({ storeId, username }) =>
    transport.checkedMutation(
      'remove_group_member',
      { storeId, username },
      decodeMutation,
    ),
  resumeGroupMemberEdit: (storeId) =>
    transport.checkedMutation(
      'resume_group_member_edit',
      { storeId },
      decodeMutation,
    ),
  addFederatedTeamMember: ({ storeId, remoteStoreId, visibility }) =>
    transport.checkedMutation(
      'add_federated_team_member',
      { storeId, remoteStoreId, visibility },
      decodeMutation,
    ),
  removeFederatedGroup: ({ storeId, remoteHostIdHex, remoteTeamIdHex }) =>
    transport.checkedMutation(
      'expel_federated_group',
      { storeId, remoteHostIdHex, remoteTeamIdHex },
      decodeMutation,
    ),
  rerunFederatedTeamMemberAdd: (storeId, operationId) =>
    transport.checkedMutation(
      'rerun_federated_team_member_add',
      { storeId, operationId },
      decodeMutation,
    ),
  configureWebAdmin: (profile, accountAlias, destination) =>
    transport.checkedMutation(
      'configure_web_admin',
      { profile, accountAlias, destination },
      decodeCommandAck,
    ),
  openWebAdmin: (profile, accountAlias, pin) =>
    transport.checkedMutation(
      'open_web_admin',
      { profile, accountAlias, pin },
      decodeCommandAck,
    ),
  botAccount: (profile, accountAlias, action) =>
    (['list', 'status'].includes(action.action)
      ? transport.checked
      : transport.checkedMutation)(
      'bot_account_request',
      { profile, accountAlias, action },
      decodeBotReply,
    ),
  setLocalAccountAlias: (store, label) =>
    transport.checkedMutation(
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
    (action === null || action.action === 'status'
      ? transport.checked
      : transport.checkedMutation)(
      'rename_account_request',
      { profile, accountAlias, action },
      decodeRenameProgress,
    ),
  adapterClock: (profile, accountAlias, confirm) =>
    (confirm ? transport.checkedMutation : transport.checked)(
      'adapter_clock_request',
      { profile, accountAlias, confirm },
      decodeAdapterClock,
    ),
  sso: (profile, accountAlias, action) =>
    (['account-status', 'status', 'poll'].includes(action.action)
      ? transport.checked
      : transport.checkedMutation)(
      'sso_request',
      { profile, accountAlias, action },
      decodeSsoProgress,
    ),
  openSsoBrowser: (profile, accountAlias, operationId) =>
    transport.checkedMutation(
      'open_sso_browser',
      { profile, accountAlias, operationId },
      decodeCopy,
    ),
  discoverGroups: (profile, accountAlias) =>
    transport.checked(
      'discover_groups',
      { profile, accountAlias },
      decodeGroupDiscovery,
    ),
});
