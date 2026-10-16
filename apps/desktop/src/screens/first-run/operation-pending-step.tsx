import type { ReactNode } from 'react';
import { Button } from '../../components';
import { SsoPanel } from '../../features/settings';
import { provisioningInFlight } from '../../first-run-operations';
import { pastePhrase } from '../../phrase-input';
import { Pane } from '../first-run-view';
import { MissingServerWarning } from './failure-text';
import { useSetupContext } from './setup-context';

/**
 * A provisioning whose outcome is not known yet: running, resumable, adoptable
 * as an existing account, or abandoned for another account.
 */
export function OperationPendingStep(): ReactNode {
  const {
    bridge,
    checkpoint,
    busy: busyWith,
    restart,
    secrets,
    account,
  } = useSetupContext();
  const busy = busyWith !== null;
  const { recoveryPhrase, setRecoveryPhrase } = secrets;
  const {
    operationRunning,
    identityLoading,
    existingAccountAdoptable,
    operationChecked,
    operationStatus,
    operationProblem,
    operationResumable,
  } = account;
  const profile = checkpoint.profile;
  const intent = checkpoint.provisioning;
  const settled =
    !operationRunning &&
    !busy &&
    !identityLoading &&
    !(intent && provisioningInFlight(bridge, intent.id));
  const adoptable = existingAccountAdoptable && settled;
  const abortable =
    operationChecked &&
    operationStatus &&
    settled &&
    restart.recoveryActions.canChooseAnotherAccount;
  const runningLabel =
    intent?.kind === 'recovery'
      ? 'Recovering…'
      : intent?.kind === 'pairing'
        ? 'Connecting…'
        : 'Creating…';
  return (
    <Pane title="Check account setup" header={false}>
      <h1>{busy ? 'Connecting your account' : 'Check account setup'}</h1>
      <p className="lead">
        {busy
          ? 'Account setup is running.'
          : operationProblem !== 'profile-missing' && operationStatus
            ? operationStatus
            : 'Checking account status…'}
      </p>
      {operationProblem === 'profile-missing' && operationStatus ? (
        <MissingServerWarning />
      ) : null}
      {operationResumable && intent?.kind === 'recovery' ? (
        <div className="local-field-card">
          <label className="local-field-row">
            <span>Recovery phrase</span>
            <input
              type="password"
              value={recoveryPhrase}
              onChange={(event) => setRecoveryPhrase(event.target.value)}
              onPaste={(event) => pastePhrase(event, setRecoveryPhrase)}
            />
          </label>
        </div>
      ) : null}
      <div className="setup-actions operation-actions">
        {adoptable ? (
          <Button
            variant="primary"
            onClick={() => void account.adoptExistingAccount()}
          >
            Continue with existing account
          </Button>
        ) : operationResumable ? (
          <Button
            variant="primary"
            disabled={
              busy || (intent?.kind === 'recovery' && !recoveryPhrase.trim())
            }
            onClick={() => void account.resume()}
          >
            Resume account setup
          </Button>
        ) : null}
        <Button
          variant={adoptable || operationResumable ? 'plain' : 'primary'}
          disabled={busy || identityLoading}
          busy={busy || identityLoading}
          onClick={() => void account.checkOperationStatus()}
        >
          {busy
            ? runningLabel
            : identityLoading
              ? 'Checking status…'
              : 'Check status'}
        </Button>
        {abortable ? (
          <Button onClick={account.discardProvisioning}>Go back</Button>
        ) : null}
      </div>
      {intent?.kind === 'sso' && intent.ssoOperationId && profile && !busy ? (
        <SsoPanel
          bridge={bridge}
          profile={profile.profile}
          account={intent.alias}
          login={false}
          deviceName={intent.deviceName}
          initialOperationId={intent.ssoOperationId}
          initialHardware={checkpoint.sso?.hardware}
          resumeOnly
          executeSignup={account.executeSsoSignup}
          onComplete={() => void account.checkOperationStatus()}
        />
      ) : null}
    </Pane>
  );
}
