import type { ReactNode } from 'react';
import { Band, Button, Inset, InsetRow, Notice } from '../../components';
import {
  completedFirstRunSteps,
  firstRunStepCount,
  type FirstRunCheckpoint,
  type FirstRunStateName,
} from '../../first-run-state';
import type { Location } from '../../location';
import { Pane, StepIntro } from '../first-run-view';

export function SetupChecklistStep({
  checkpoint,
  personalRefreshError,
  admin,
  adminShort,
  group,
  accountStore,
  personalRefreshing,
  retryPersonal,
  onNavigate,
  go,
  onCopy,
}: {
  checkpoint: FirstRunCheckpoint;
  personalRefreshError: string | null;
  admin: string;
  adminShort: string;
  group: string;
  accountStore: string | undefined;
  personalRefreshing: boolean;
  retryPersonal: () => void;
  onNavigate: (location: Location) => void;
  go: (state: FirstRunStateName) => void;
  onCopy: (value: string) => void;
}): ReactNode {
  const profile = checkpoint.profile;
  const stepsDone = completedFirstRunSteps(checkpoint);
  const stepsTotal = firstRunStepCount(checkpoint);
  const recoverySet = Boolean(
    checkpoint.passphraseSet || checkpoint.backupCommitted,
  );
  // Completed checklist items display one summary line; server connection
  // details are available in Settings. If recovery setup was skipped, the
  // warning banner explains that unbacked accounts cannot be recovered.
  return (
    <Pane title="Get started" header={false}>
      <StepIntro>
        <h1>Get started</h1>
        <p className="lead">
          {stepsDone === stepsTotal
            ? 'Your account is ready. Start using your Personal vault.'
            : 'Completed steps are saved. Finish account recovery below, or start using your Personal vault.'}
        </p>
      </StepIntro>
      {personalRefreshError ? (
        <p className="crit" role="alert">
          {personalRefreshError}
        </p>
      ) : null}
      <Inset className="checklist">
        <InsetRow label="✓">
          <b>
            {checkpoint.path === 'invited' ? 'Their server' : 'Select a server'}
          </b>
          <span className="hint">Using {profile?.canonicalName}</span>
        </InsetRow>
        <InsetRow label="✓">
          <b>Your account</b>
          <span className="hint">
            {checkpoint.accountMethod === 'recover'
              ? 'Restored account as'
              : 'Created as'}{' '}
            {checkpoint.account?.username}
          </span>
        </InsetRow>
        <InsetRow
          className={recoverySet ? undefined : 'skipped'}
          label={recoverySet ? '✓' : '!'}
        >
          <b>Save recovery phrase</b>
          <span className="hint">
            {recoverySet
              ? [
                  checkpoint.backupCommitted ? 'Recovery phrase saved' : null,
                  checkpoint.passphraseSet ? 'Passphrase set' : null,
                ]
                  .filter(Boolean)
                  .join(' · ')
              : 'Recovery phrase not saved'}
          </span>
        </InsetRow>
        {checkpoint.path === 'invited' ? (
          <InsetRow
            className={checkpoint.added ? undefined : 'pending'}
            label={checkpoint.added ? '✓' : '4'}
            action={
              <span className="checklist-actions">
                <Button
                  size="sm"
                  icon="copy"
                  onClick={() =>
                    onCopy(
                      `Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`,
                    )
                  }
                >
                  Copy Message
                </Button>
                <Button size="sm" onClick={() => go('waiting')}>
                  Check Now
                </Button>
              </span>
            }
          >
            <b>Waiting for {admin} to add you</b>
            <span className="hint">
              Check again after {adminShort} adds you to {group}.
            </span>
            <Band label="Team discovery">
              Check Now looks up teams for this signed-in account.
            </Band>
          </InsetRow>
        ) : null}
      </Inset>
      {recoverySet ? null : (
        <div className="checklist-notice">
          {/* A notice, not a band: its title is a statement of its own
              and the sentence under it explains it. */}
          <Notice
            title="Recovery not set up"
            actions={
              <Button variant="primary" onClick={() => go('protect')}>
                Set Up Recovery
              </Button>
            }
          >
            <p>
              The keys for {checkpoint.account?.username} are saved only on this
              device, so this account cannot be recovered if the device is lost.
            </p>
          </Notice>
        </div>
      )}
      <div className="checklist-cta">
        {!accountStore ? (
          <Button disabled={personalRefreshing} onClick={retryPersonal}>
            {personalRefreshing
              ? 'Loading Personal Vault…'
              : 'Retry Loading Personal Vault'}
          </Button>
        ) : null}
        <Button
          variant="primary"
          disabled={!accountStore}
          onClick={() => {
            if (accountStore) onNavigate({ kind: 'store', ref: accountStore });
          }}
        >
          Continue to My Vault
        </Button>
      </div>
    </Pane>
  );
}
