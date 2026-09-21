import type { Bridge, YubiCommand } from '../../bridge';
import type { AccountStore } from '../../model';
import type { SimpleYubiAction, PassphraseMode } from '../device-sheets';
import type { useWorkflowAccess } from '../../workflow-context';

export function credentialCommand({
  action,
  profile,
  alias,
  store,
  pin,
  other,
  confirmation,
}: {
  action: SimpleYubiAction;
  profile: string;
  alias: string;
  store?: AccountStore;
  pin: string;
  other: string;
  confirmation: string;
}): YubiCommand | undefined {
  switch (action) {
    case 'sync':
      return {
        command: 'sync_yubi_account',
        args: { profile, alias, pin, withFederation: true },
      };
    case 'pin-status':
      return { command: 'yubi_pin_status', args: { profile, alias } };
    case 'change-pin':
      return {
        command: 'change_yubi_pin',
        args: { profile, alias, oldPin: pin, newPin: other },
      };
    case 'set-passphrase':
    case 'change-passphrase':
      return {
        command:
          action === 'set-passphrase'
            ? 'set_yubi_passphrase'
            : 'change_yubi_passphrase',
        args: { profile, alias, pin, passphrase: other, confirmation },
      };
    case 'verify-passphrase':
      return {
        command: 'verify_yubi_passphrase',
        args: { profile, alias, pin, passphrase: other },
      };
    case 'unblock':
      return {
        command: 'unblock_yubi_pin',
        args: { profile, alias, puk: pin, newPin: other },
      };
    case 'change-puk':
      return {
        command: 'change_yubi_puk',
        args: { profile, alias, oldPuk: pin, newPuk: other },
      };
    case 'recover-management':
      if (!store) return;
      return {
        command: 'recover_yubi_management_key',
        args: { accountStoreId: store.id, yubiAlias: alias },
      };
    case 'recover-subkey':
      return { command: 'recover_yubi_subkey', args: { profile, alias, pin } };
    case 'resume-enrollment':
      return { command: 'resume_yubi_account', args: { profile, alias, pin } };
    case 'resume-rotation':
      return {
        command: 'resume_yubi_management_key',
        args: { profile, alias, ...(pin ? { pin } : {}) },
      };
    case 'rotate':
      return {
        command: 'rotate_yubi_management_key',
        args: { profile, alias, pin },
      };
  }
}

/** `current` is the passphrase to check before a change is submitted. It is
 * null for an enrollment, which has none, and for the device-authorized
 * change that replaces a forgotten one. */
export function accountPassphrase(
  bridge: Bridge,
  access: ReturnType<typeof useWorkflowAccess>,
  store: AccountStore,
  mode: PassphraseMode,
  current: string | null,
  secret: string,
  repeated: string,
) {
  return access.run(
    'passphrase',
    {
      profile: store.server,
      account: store.account,
    },
    () =>
      mode === 'set'
        ? bridge.setAccountPassphrase(store.id, secret, repeated)
        : bridge.changeAccountPassphrase(store.id, current, secret, repeated),
  );
}

export function accountPassphraseStatus(
  bridge: Bridge,
  access: ReturnType<typeof useWorkflowAccess>,
  store: AccountStore,
) {
  return access.run(
    'passphrase',
    {
      profile: store.server,
      account: store.account,
    },
    () => bridge.accountPassphraseStatus(store.id),
  );
}
