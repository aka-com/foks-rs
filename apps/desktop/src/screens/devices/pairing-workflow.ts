import type { Bridge } from '../../bridge';
import type { AccountStore } from '../../model';
import type { useWorkflowAccess } from '../../workflow-context';
import { queuedDeviceWork } from './operation-controller';

export async function finishPairing(bridge: Bridge, store: AccountStore) {
  const result = await bridge.finishDevicePairing(store.id);
  if (result.alias !== store.account)
    throw new Error('finish_device_pairing returned a different account.');
  return result;
}

export async function acceptPairing(
  bridge: Bridge,
  profile: string,
  target: string,
  submission: { device: string; phrase: string } | 'resume',
) {
  const result = submission === 'resume'
    ? await bridge.resumeDevicePairingAcceptance(profile, target)
    : await bridge.acceptDevicePairing(profile, target, submission.device, submission.phrase);
  if (result.alias !== target)
    throw new Error(`${submission === 'resume'
      ? 'resume_device_pairing_acceptance'
      : 'accept_device_pairing'} returned a different account.`);
  return result;
}

export function recoverAccount(
  bridge: Bridge,
  access: ReturnType<typeof useWorkflowAccess>,
  profile: string,
  target: string,
  phrase: string,
  device: string,
) {
  return queuedDeviceWork(
    bridge,
    profile,
    access,
    'account-recover',
    { profile },
    () => bridge.recoverOwnerAccount(profile, target, phrase, device),
  );
}
