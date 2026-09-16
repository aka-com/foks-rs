import type { ReactNode } from 'react';
import { Button, Icon } from '../components';
import { storeDescription, plural, type AgentSnapshot } from '../model';
import type { Location } from '../location';
import type { CheckedProfileResponse } from '../bridge';
import { Pane, Foot } from './first-run-view';
interface Props {
  personalAvailable: boolean;
  accountStoreRecord: AgentSnapshot['stores'][number] | undefined;
  accountStore: string | undefined;
  personalRefreshing: boolean;
  retryPersonal: () => void;
  onNavigate: (location: Location) => void;
  profile: CheckedProfileResponse | undefined;
  snapshot: AgentSnapshot;
  accountItemCount: number;
  personalRefreshError: string | null;
}
export function LocalCompleteStep({
  personalAvailable,
  accountStoreRecord,
  accountStore,
  personalRefreshing,
  retryPersonal,
  onNavigate,
  profile,
  snapshot,
  accountItemCount,
  personalRefreshError,
}: Props): ReactNode {
  return (
    <Pane
      title="Ready"
      subtitle="Setup complete"
      header={false}
      foot={
        <Foot>
          {!personalAvailable ? (
            <>
              {accountStoreRecord ? (
                <Button
                  onClick={() =>
                    onNavigate({
                      kind: 'settings',
                      section: 'servers',
                      profile: profile?.profile,
                    })
                  }
                >
                  Review server settings
                </Button>
              ) : null}
              <Button
                variant="primary"
                disabled={personalRefreshing}
                onClick={retryPersonal}
              >
                {personalRefreshing
                  ? 'Loading Personal vault…'
                  : 'Retry loading Personal vault'}
              </Button>
            </>
          ) : (
            <Button
              variant="primary"
              disabled={!accountStore}
              onClick={() => {
                if (accountStore)
                  onNavigate({ kind: 'store', ref: accountStore });
              }}
            >
              Open Personal
            </Button>
          )}
        </Foot>
      }
    >
      <div className="local-success">
        <span className="local-success-mark" aria-hidden="true">
          ✓
        </span>
        <h1>
          {personalAvailable
            ? 'Your Personal vault is ready'
            : 'Setup is complete'}
        </h1>
        <p className="lead">
          {personalAvailable
            ? 'Your account is connected to the local server on this device.'
            : accountStoreRecord
              ? `Your Personal vault is unavailable (${storeDescription(snapshot, accountStoreRecord).toLowerCase()}). Your setup progress is saved. Check server settings to restore access.`
              : 'FOKS could not load your Personal vault. Your setup progress is saved. Retry loading the vault to continue.'}
        </p>
        {personalRefreshError ? (
          <p className="crit" role="alert">
            {personalRefreshError}
          </p>
        ) : null}
        <div className="local-vault-preview">
          <div className="local-vault-head">
            <Icon name="vault" />
            <b>Personal</b>
            <code>{profile?.canonicalName}</code>
          </div>
          <div className="local-vault-empty">
            {!personalAvailable
              ? 'Personal vault unavailable'
              : accountItemCount === 0
                ? 'No items yet'
                : plural(accountItemCount, 'item')}
          </div>
        </div>
      </div>
    </Pane>
  );
}
