import type { AppInfo, Bridge } from '../bridge';
import type { MetadataQuery, MetadataRepository } from '../metadata-repository';

export const appInfoKey = ['application-info'] as const;

/**
 * How long the application facts are current. Nothing this app does changes
 * them: the version is the running build's, the agent socket is chosen once
 * at startup, and the managed profile comes from the launcher's environment.
 * An agent or access change retires the whole repository, and a manual
 * refresh discards every row, so neither is waited out here.
 */
export const APP_INFO_FRESHNESS = 5 * 60_000;

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
    APP_INFO_FRESHNESS,
  );
}
