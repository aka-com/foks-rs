import type { AppInfo, Bridge } from '../bridge';
import type { MetadataQuery, MetadataRepository } from '../metadata-repository';

export const appInfoKey = ['application-info'] as const;

export function appInfoQuery(
  repository: MetadataRepository,
  bridge: Pick<Bridge, 'appInfo'>,
): MetadataQuery<AppInfo> {
  return repository.query(
    appInfoKey,
    async () => {
      const { version, agentSocket, managedProfile, computerName, userName } =
        await bridge.appInfo();
      return {
        version,
        agentSocket,
        ...(managedProfile === undefined ? {} : { managedProfile }),
        ...(computerName === undefined ? {} : { computerName }),
        ...(userName === undefined ? {} : { userName }),
      };
    },
    60_000,
  );
}
