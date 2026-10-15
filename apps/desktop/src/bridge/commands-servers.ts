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
import type { Transport } from './transport';

export const serverCommands = (
  transport: Transport,
): Pick<
  Bridge,
  | 'listServers'
  | 'checkAndAddProfile'
  | 'checkAndAddGoProfile'
  | 'describeServerStatus'
  | 'checkServer'
  | 'reconcileServer'
  | 'setServerLabel'
  | 'removeServerAndCredentials'
> => ({
  listServers: (generation) =>
    transport.checked(
      'list_servers',
      generation === undefined ? undefined : { generation },
      decodeServers,
    ),
  checkAndAddProfile: (profileName, probe) =>
    transport.checkedMutation(
      'check_and_add_profile',
      { profileName, probe },
      decodeCheckedProfile,
    ),
  checkAndAddGoProfile: (candidateId, hostId, profileName, probe) =>
    transport.checkedMutation(
      'check_and_add_go_profile',
      { candidateId, hostId, profileName, probe },
      decodeCheckedProfile,
    ),
  describeServerStatus: (profile, fresh) =>
    transport.checked(
      'describe_server_status',
      { profile, ...(fresh ? { fresh: true } : {}) },
      decodeServerStatus,
    ),
  checkServer: (profile) =>
    transport.checkedMutation('check_server', { profile }, decodeCheckedServer),
  reconcileServer: (profile) =>
    transport.checked('reconcile_server', { profile }, (value) =>
      decodeProfileReconciliation(value, profile),
    ),
  setServerLabel: (profile, label) =>
    transport.checkedMutation(
      'set_server_label',
      { profile, label },
      decodeServerLabelResponse,
    ),
  removeServerAndCredentials: (profileName, confirmedProfileName) =>
    transport.checkedMutation(
      'remove_server_and_credentials',
      { profileName, confirmedProfileName },
      decodeRemovedServer,
    ),
});
