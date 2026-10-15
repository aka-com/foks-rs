import {
  decodeInvitationReply,
  invitationActionMutates,
} from '../invitation-contract';
import { accountCommands } from './commands-accounts';
import { coreCommands } from './commands-core';
import { enrollmentCommands } from './commands-enrollment';
import { serverCommands } from './commands-servers';
import { vaultCommands } from './commands-vault';
import { yubiCommands } from './commands-yubikey';
import type { Bridge } from './contract';
import { enqueueProfileWork } from './profile-work';
import type { Transport } from './transport';

/**
 * The bridge whose commands go through `transport`. `overrides` replaces
 * methods on the returned object itself, so the profile queue that
 * `invitation` joins is keyed by the bridge its callers hold.
 */
export function createBridge(
  transport: Transport,
  overrides: Partial<Bridge> = {},
): Bridge {
  const bridge: Bridge = {
    native: true,
    ...accountCommands(transport),
    ...coreCommands(transport),
    ...enrollmentCommands(transport),
    ...serverCommands(transport),
    ...vaultCommands(transport),
    ...yubiCommands(transport),
    // This method owns its profile admission: it is the one bridge method
    // that queues itself, because its callers issue several invitation reads
    // at once and the agent admits one per profile. A caller must not queue
    // it again: the outer entry would hold the profile's slot while awaiting
    // an entry that cannot start until the slot is released, and the inner
    // one would expire at the admission deadline instead of running. The
    // test `profile-queue-ownership.test.ts` holds this contract.
    invitation: (profile, accountAlias, action, pin) =>
      enqueueProfileWork(
        bridge,
        profile,
        () =>
          (invitationActionMutates(action)
            ? transport.checkedMutation
            : transport.checked)(
            'invitation_request',
            { profile, accountAlias, action, pin },
            decodeInvitationReply,
          ),
        'invitation',
      ),
    ...overrides,
  };
  return bridge;
}
