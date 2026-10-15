/**
 * The hardware-key and passphrase commands the credential sheets run, with
 * the title each YubiKey sheet reads and the workflow whose availability
 * gates it.
 */

import type { WorkflowOperation } from '../../model/workflow-availability';

/** A YubiKey command that is one sheet of typed values. */
export type SimpleYubiAction =
  | 'sync'
  | 'pin-status'
  | 'change-pin'
  | 'set-passphrase'
  | 'change-passphrase'
  | 'verify-passphrase'
  | 'unblock'
  | 'change-puk'
  | 'recover-management'
  | 'recover-subkey'
  | 'resume-enrollment'
  | 'resume-rotation'
  | 'rotate';

/** What the passphrase sheet does with what is typed into it. */
export type PassphraseMode = 'set' | 'change';

/** The label a YubiKey sheet's title reads, by the action it performs. */
export const YUBI_ACTION_LABELS: Readonly<Record<SimpleYubiAction, string>> = {
  sync: 'Sync the key account',
  'pin-status': 'Card PIN status',
  'change-pin': 'Change the card PIN',
  'set-passphrase': 'Set the key passphrase',
  'change-passphrase': 'Change the key passphrase',
  'verify-passphrase': 'Verify the key passphrase',
  unblock: 'Unblock the card PIN',
  'change-puk': 'Change the unlock code',
  'recover-management': 'Restore management access',
  'recover-subkey': 'Restore the signing key',
  'resume-enrollment': 'Resume enrollment',
  'resume-rotation': 'Resume key rotation',
  rotate: 'Rotate the management key',
};

export const YUBI_WORKFLOWS: Readonly<
  Record<SimpleYubiAction, WorkflowOperation>
> = {
  sync: 'account-sync',
  'pin-status': 'yubi-pin',
  'change-pin': 'yubi-pin',
  'set-passphrase': 'passphrase',
  'change-passphrase': 'passphrase',
  'verify-passphrase': 'passphrase',
  unblock: 'yubi-pin',
  'change-puk': 'yubi-pin',
  'recover-management': 'yubi-recover-management',
  'recover-subkey': 'yubi-recover-subkey',
  'resume-enrollment': 'yubi-resume',
  'resume-rotation': 'yubi-rotate',
  rotate: 'yubi-rotate',
};
