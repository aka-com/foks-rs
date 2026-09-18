import { enqueueProfileWork } from '../../bridge';
import type { BackupEnrollment, Bridge } from '../../bridge';
import type { AccountStore } from '../../model';
import type { useWorkflowAccess } from '../../workflow-context';

type Access = ReturnType<typeof useWorkflowAccess>;

export async function revokeSecurityKey(
  bridge: Bridge,
  access: Access,
  store: AccountStore,
  alias: string,
  confirmation: string,
) {
  const result = await access.run(
    'yubi-revoke',
    {
      profile: store.server,
      account: store.account,
    },
    () =>
      bridge.runYubi({
        command: 'revoke_yubi_device',
        args: { accountStoreId: store.id, yubiAlias: alias, confirmation },
      }),
  );
  if (result.alias !== alias || result.removedLocalCredential !== true)
    throw new Error('revoke_yubi_device returned a different enrollment.');
  return result;
}

export async function revokePaperKey(
  bridge: Bridge,
  access: Access,
  store: AccountStore,
  backup: BackupEnrollment,
  confirmation: string,
) {
  const revoked = await access.run(
    'backup-revoke',
    {
      profile: store.server,
      account: store.account,
    },
    () => bridge.revokeOwnerBackup(store.id, backup, confirmation),
  );
  if (
    revoked.backupAlias !== backup.backupAlias ||
    revoked.backupId !== backup.backupId
  )
    throw new Error('revoke_owner_backup returned a different enrollment.');
  return revoked;
}

export async function removeDevice(
  bridge: Bridge,
  access: Access,
  store: AccountStore,
  deviceId: string,
) {
  const target = { profile: store.server, account: store.account };
  const removed = await enqueueProfileWork(bridge, store.server, async () => {
    access.require('devices-list', target);
    await bridge.listAccountDevices(store.id);
    return access.run('device-remove', target, () =>
      bridge.removeAccountDevice(store.id, deviceId),
    );
  });
  if (removed.deviceId !== deviceId)
    throw new Error('remove_account_device returned a different device.');
  return removed;
}
