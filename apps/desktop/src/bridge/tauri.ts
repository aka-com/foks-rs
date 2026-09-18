import { decodeInvitationReply } from '../invitation-contract';
import { accountCommands } from './commands-accounts';
import { coreCommands } from './commands-core';
import { enrollmentCommands } from './commands-enrollment';
import { serverCommands } from './commands-servers';
import { vaultCommands } from './commands-vault';
import { yubiCommands } from './commands-yubikey';
import type { Bridge } from './contract';
import { enqueueProfileWork } from './profile-work';
import { checked } from './transport';

export const tauriBridge: Bridge = {
  native: true,
  ...accountCommands,
  ...coreCommands,
  ...enrollmentCommands,
  ...serverCommands,
  ...vaultCommands,
  ...yubiCommands,
  invitation: (profile, accountAlias, action, pin) =>
    enqueueProfileWork(tauriBridge, profile, () =>
      checked(
        'invitation_request',
        { profile, accountAlias, action, pin },
        decodeInvitationReply,
      ),
    ),
};
