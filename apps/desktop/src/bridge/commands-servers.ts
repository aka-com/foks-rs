import type { Bridge } from './contract';
import {
  decodeCheckedProfile,
  decodeCheckedServer,
  decodeRemovedServer,
  decodeProfileReconciliation,
  decodeServerLabelResponse,
  decodeServers,
  decodeServerStatus,
} from './servers';
import { checked, checkedMutation } from './transport';

export const serverCommands: Pick<
  Bridge,
  | 'listServers'
  | 'checkAndAddProfile'
  | 'checkAndAddGoProfile'
  | 'describeServerStatus'
  | 'checkServer'
  | 'reconcileServer'
  | 'setServerLabel'
  | 'removeServerAndCredentials'
> = {
  listServers: (generation) =>
    checked(
      'list_servers',
      generation === undefined ? undefined : { generation },
      decodeServers,
    ),
  checkAndAddProfile: (profileName, probe) =>
    checkedMutation(
      'check_and_add_profile',
      { profileName, probe },
      decodeCheckedProfile,
    ),
  checkAndAddGoProfile: (candidateId, hostId, profileName, probe) =>
    checkedMutation(
      'check_and_add_go_profile',
      { candidateId, hostId, profileName, probe },
      decodeCheckedProfile,
    ),
  describeServerStatus: (profile, fresh) =>
    checked(
      'describe_server_status',
      { profile, ...(fresh ? { fresh: true } : {}) },
      decodeServerStatus,
    ),
  checkServer: (profile) =>
    checkedMutation('check_server', { profile }, decodeCheckedServer),
  reconcileServer: (profile) =>
    checked('reconcile_server', { profile }, (value) =>
      decodeProfileReconciliation(value, profile),
    ),
  setServerLabel: (profile, label) =>
    checkedMutation(
      'set_server_label',
      { profile, label },
      decodeServerLabelResponse,
    ),
  removeServerAndCredentials: (profile, confirmation) =>
    checkedMutation(
      'remove_server_and_credentials',
      { profile, confirmation },
      decodeRemovedServer,
    ),
};
