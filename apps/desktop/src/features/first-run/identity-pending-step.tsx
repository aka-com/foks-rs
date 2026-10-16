import type { ReactNode } from 'react';
import { Button } from '../../components';
import { Pane } from './first-run-view';
import { MissingServerWarning } from './failure-text';
import { useSetupContext } from './setup-context';

/** An acknowledged provisioning whose account details have not loaded yet. */
export function IdentityPendingStep(): ReactNode {
  const { checkpoint, agentReady, onNavigate, account } = useSetupContext();
  const {
    identityProblem,
    identityError,
    identityWaiting,
    identityLoading,
    discardProvisionedAccount,
  } = account;
  return (
    <Pane title="Load account details" header={false}>
      <h1>Your account is connected</h1>
      <p className="lead">
        Your account is connected. Try again to load its details, or finish
        setup later.
      </p>
      {identityProblem === 'profile-missing' ? (
        <MissingServerWarning
          action={
            <Button onClick={discardProvisionedAccount}>
              Set up a different account
            </Button>
          }
        />
      ) : identityError ? (
        <p className="crit" role="alert">
          {identityError}
        </p>
      ) : identityWaiting ? (
        <p className="status" role="status">
          {identityWaiting}
        </p>
      ) : null}
      <div className="setup-actions">
        <Button
          variant="primary"
          disabled={identityLoading || !agentReady}
          busy={identityLoading}
          onClick={() => void account.refreshAccountIdentity()}
        >
          {identityLoading
            ? 'Loading account details…'
            : 'Retry loading account'}
        </Button>
        <Button
          onClick={() =>
            onNavigate({
              kind: 'settings',
              section: 'account',
              profile: checkpoint.profile?.profile,
            })
          }
        >
          Review server settings
        </Button>
      </div>
      {identityProblem &&
      identityProblem !== 'profile-missing' &&
      !identityLoading ? (
        <div className="alt-path">
          <div className="t">
            <b>Set up a different account</b>
            <span>
              Your existing accounts and server settings will be kept.
            </span>
          </div>
          <Button onClick={discardProvisionedAccount}>
            Set up a different account
          </Button>
        </div>
      ) : null}
    </Pane>
  );
}
