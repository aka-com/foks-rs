import type { Bridge } from './contract';
import {
  decodeAddedServer,
  decodeCheckedProfile,
  decodeCheckedServer,
  decodeForgottenServer,
  decodeProfileReconciliation,
  decodeServerLabelResponse,
  decodeServers,
  decodeServerStatus,
} from './servers';
import { checked } from './transport';

export const serverCommands: Pick<
  Bridge,
  | 'listServers'
  | 'checkAndAddProfile'
  | 'checkAndAddGoProfile'
  | 'describeServerStatus'
  | 'checkServer'
  | 'reconcileServer'
  | 'addServer'
  | 'setServerLabel'
  | 'forgetServer'
> = {
  listServers: (generation) =>
    checked(
      'list_servers',
      generation === undefined ? undefined : { generation },
      decodeServers,
    ),
  checkAndAddProfile: (profileName, probe) =>
    checked(
      'check_and_add_profile',
      { profileName, probe },
      decodeCheckedProfile,
    ),
  checkAndAddGoProfile: (candidateId, hostId, profileName, probe) =>
    checked(
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
    checked('check_server', { profile }, decodeCheckedServer),
  reconcileServer: (profile) =>
    checked('reconcile_server', { profile }, (value) =>
      decodeProfileReconciliation(value, profile),
    ),
  addServer: (profileName, probe) =>
    checked('add_server', { profileName, probe }, decodeAddedServer),
  setServerLabel: (profile, label) =>
    checked('set_server_label', { profile, label }, decodeServerLabelResponse),
  forgetServer: (profile, confirmation) =>
    checked('forget_server', { profile, confirmation }, decodeForgottenServer),
};
