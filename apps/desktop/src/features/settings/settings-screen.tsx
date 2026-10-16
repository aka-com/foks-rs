/**
 * The Settings tab: a sub-navigation of three pages, held on screen beside
 * whichever one is open.
 *
 * Account, the first page and the one the tab opens on, is one account's
 * profile: the account the address's `store` names. At its bottom are the
 * servers this Mac talks to. Each server opens its own page, where its security
 * keys are managed. Preferences contains local desktop alert and appearance settings.
 * Device contains the application version and lock, the agent and its socket,
 * local FOKS data operations, and the device-wide reset. A `section=` address
 * opens its page; `profile=` opens a server page under Account.
 *
 * Account draws its own header — the account's mark, username and server —
 * and its own tab panel, and owns its sheets. The other two share the
 * header and panel drawn here.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '../../../kit/toasts';
import { Tabs, tabId, tabPanelId } from '../../components';
import {
  SETTINGS_SECTIONS,
  SETTINGS_SECTION_LABEL,
  settingsSectionOf,
} from '../../location';
import type {
  Location,
  NavigateOptions,
  SettingsSection,
} from '../../location';
import {
  useMetadataQuery,
  useMetadataRepository,
} from '../../data/query-hooks';
import { appInfoQuery } from '../../resources/application';
import { plural } from '../../model';
import { PageHeader } from '../../shell/page-header';
import { useShellServices, useShellSnapshot } from '../../shell-services';
import { ServersSection } from './servers/servers-screen';
import { AccountSection } from './account/account-section';
import type { AgentLifecycle } from '../../agent-lifecycle';
import { PreferencesSection } from './preferences-section';
import { DeviceSection } from './device-section';
import { ResetMacSheet } from './reset-mac-sheet';

/**
 * The page reads the snapshot, the bridge, the catalog refreshes and the error
 * handlers from the shell services.
 */
export interface SettingsScreenProps {
  location: Extract<Location, { kind: 'settings' }>;
  scene: string;
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
}

/** A sheet this page owns. Add a server and server removal are the
 *  embedded server section's own. */
type Sheet = 'reset-mac' | null;

/** The base the sub-navigation's tab and panel ids are derived from. */
const SETTINGS_TABS = 'settings-sections';

export function SettingsScreen({
  location,
  scene,
  onNavigate,
  onLock,
  agentLifecycle,
  onRetryAgent,
}: SettingsScreenProps): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onRefresh,
    refreshSnapshot: onRefreshSnapshot,
    commandError: onError,
    mutationError: onMutationError,
  } = useShellServices();
  const [enteredScene] = useState(scene);
  const toasts = useToast();
  const [sheet, setSheet] = useState<Sheet>(null);
  const repository = useMetadataRepository(bridge);
  const { data: appInfo } = useMetadataQuery(appInfoQuery(repository, bridge), {
    onError,
  });

  const section: SettingsSection = settingsSectionOf(location);

  const serverPanel = {
    id: tabPanelId(SETTINGS_TABS, 'account'),
    labelledBy: tabId(SETTINGS_TABS, 'account'),
  };
  const selectedServer = location.profile
    ? snapshot.servers.find((server) => server.profileName === location.profile)
    : undefined;
  const serversSection = (
    <ServersSection
      snapshot={snapshot}
      bridge={bridge}
      store={location.store}
      scene={enteredScene}
      onNavigate={onNavigate}
      onRefresh={onRefresh}
      onError={onError}
      onMutationError={onMutationError}
    />
  );

  const page: ReactNode =
    section === 'account' ? null : section === 'preferences' ? (
      <PreferencesSection />
    ) : (
      <DeviceSection
        snapshot={snapshot}
        bridge={bridge}
        appInfo={appInfo ?? null}
        onError={onError}
        onMessage={(text: string) => toasts.show(text)}
        onLock={onLock}
        agentLifecycle={agentLifecycle}
        onRetryAgent={onRetryAgent}
        serversCount={snapshot.servers.length}
        onReset={() => setSheet('reset-mac')}
      />
    );

  return (
    <div className="settingslayout">
      <nav className="side subnav-side" aria-label="Settings sections">
        <Tabs
          orientation="vertical"
          label="Settings sections"
          idBase={SETTINGS_TABS}
          value={section}
          onChange={(next) => {
            // Settings sections are sibling views within this tab and replace
            // current content without adding history entries. Selecting a
            // section opens its root view without a server profile.
            onNavigate(
              {
                kind: 'settings',
                ...(location.store ? { store: location.store } : {}),
                section: next,
              },
              { replace: true },
            );
          }}
          items={SETTINGS_SECTIONS.map((id) => ({
            id,
            label: SETTINGS_SECTION_LABEL[id],
          }))}
        />
      </nav>
      <div className="subnav-page">
        {section === 'account' && selectedServer ? (
          <ServersSection
            snapshot={snapshot}
            bridge={bridge}
            profile={selectedServer.profileName}
            store={location.store}
            scene={enteredScene}
            panel={serverPanel}
            onNavigate={onNavigate}
            onRefresh={onRefresh}
            onError={onError}
            onMutationError={onMutationError}
          />
        ) : section === 'account' ? (
          <AccountSection
            snapshot={snapshot}
            bridge={bridge}
            location={location}
            panel={serverPanel}
            onNavigate={onNavigate}
            onRefresh={onRefresh}
            onRefreshSnapshot={onRefreshSnapshot}
            onError={onError}
            onMutationError={onMutationError}
            serverSection={serversSection}
          />
        ) : (
          <>
            <PageHeader ruled title={SETTINGS_SECTION_LABEL[section]} />
            {/* Fixed IDs associate each tab button with its tabpanel. */}
            <div
              className="body"
              role="tabpanel"
              id={tabPanelId(SETTINGS_TABS, section)}
              aria-labelledby={tabId(SETTINGS_TABS, section)}
            >
              <div className="settings-main">{page}</div>
            </div>
          </>
        )}
      </div>
      {sheet === 'reset-mac' ? (
        <ResetMacSheet
          snapshot={snapshot}
          bridge={bridge}
          onClose={() => setSheet(null)}
          onDone={async (count) => {
            setSheet(null);
            await onRefreshSnapshot();
            await onRefresh(
              `Local state reset for ${plural(count, 'server')}. Verify each server before reconnecting.`,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
    </div>
  );
}
