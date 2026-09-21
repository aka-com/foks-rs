import { decodeInvitationReply } from '../invitation-contract';
import { accountCommands } from './commands-accounts';
import { coreCommands } from './commands-core';
import { enrollmentCommands } from './commands-enrollment';
import { serverCommands } from './commands-servers';
import { vaultCommands } from './commands-vault';
import { yubiCommands } from './commands-yubikey';
import type { Bridge } from './contract';
import { enqueueProfileWork } from './profile-work';
import { checked, checkedMutation } from './transport';

export const tauriBridge: Bridge = {
  native: true,
  ...accountCommands,
  ...coreCommands,
  ...enrollmentCommands,
  ...serverCommands,
  ...vaultCommands,
  ...yubiCommands,
  // This method owns its profile admission: it is the one bridge method that
  // queues itself, because its callers issue several invitation reads at
  // once and the agent admits one per profile. A caller must not queue it
  // again: the outer entry would hold the profile's slot while awaiting an
  // entry that cannot start until the slot is released, and the inner one
  // would expire at the admission deadline instead of running. The test
  // `profile-queue-ownership.test.ts` holds this contract.
  invitation: (profile, accountAlias, action, pin) =>
    enqueueProfileWork(tauriBridge, profile, () =>
      ([
        'preview',
        'preview-remote',
        'inbox',
        'pending-approvals',
        'list',
        'status',
        'status-remote',
        'inspect-remote',
      ].includes(action.action)
        ? checked
        : checkedMutation)(
        'invitation_request',
        { profile, accountAlias, action, pin },
        decodeInvitationReply,
      ),
    ),
};
