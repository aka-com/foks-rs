import type { ReactNode } from 'react';
import {
  Button,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  Toggle,
} from '../../components';
import { fixDeviceName } from '../../device-name';
import type {
  FirstRunCheckpoint,
  FirstRunStateName,
} from '../../first-run-state';
import { Foot, Pane } from '../first-run-view';
import { StepLabel } from './step-label';

/** Account choice and form layout; provisioning and SSO ownership stay in the session. */
export function AccountSetupStep({
  checkpoint,
  signingIn,
  cliAccountSelected,
  ssoAvailable,
  ssoSelected,
  busy,
  usernameAliasInvalid,
  username,
  deviceName,
  signupPending,
  email,
  invite,
  goCandidatePresent,
  accountBackTarget,
  identityLoading,
  duplicateAlias,
  message,
  identityRows,
  aliasInvalidNotice,
  signinPrimary,
  signinSections,
  organizationSignup,
  failureText,
  go,
  createAccount,
  openExisting,
  setEmail,
  setInvite,
  onChooseOrganization,
  onPrimarySlot,
  adoptDuplicateAccount,
}: {
  checkpoint: FirstRunCheckpoint;
  signingIn: boolean;
  cliAccountSelected: boolean;
  ssoAvailable: boolean;
  ssoSelected: boolean;
  busy: boolean;
  usernameAliasInvalid: boolean;
  username: string;
  deviceName: string;
  signupPending: boolean;
  email: string;
  invite: string;
  goCandidatePresent: boolean;
  accountBackTarget: FirstRunStateName;
  identityLoading: boolean;
  duplicateAlias: { alias: string } | null;
  message: string | null;
  identityRows: ReactNode;
  aliasInvalidNotice: ReactNode;
  signinPrimary: ReactNode;
  signinSections: (first: number) => ReactNode;
  organizationSignup: ReactNode;
  failureText: ReactNode;
  go: (state: FirstRunStateName) => void;
  createAccount: () => Promise<void>;
  openExisting: () => void;
  setEmail: (value: string) => void;
  setInvite: (value: string) => void;
  onChooseOrganization: () => void;
  onPrimarySlot: (element: HTMLElement | null) => void;
  adoptDuplicateAccount: () => Promise<void>;
}): ReactNode {
  const profile = checkpoint.profile;
  return (
    <Pane
      title="Your account"
      header={false}
      scope={
        signingIn
          ? 'Device authorization required'
          : 'Keys are generated securely on your device.'
      }
      wide
      foot={
        <Foot
          back={() =>
            go(
              signingIn
                ? checkpoint.account
                  ? 'protect'
                  : accountBackTarget
                : accountBackTarget,
            )
          }
        >
          {signingIn ? (
            checkpoint.account ? (
              <Button onClick={() => go('protect')}>Skip to latest step</Button>
            ) : (
              signinPrimary
            )
          ) : ssoSelected ? (
            <span className="foot-slot" ref={onPrimarySlot} />
          ) : (
            <Button
              variant="primary"
              disabled={
                busy ||
                usernameAliasInvalid ||
                (!checkpoint.account &&
                  (!username.trim() || !fixDeviceName(deviceName)))
              }
              // If the account was already created when returning from Protect, proceed to protection.
              onClick={() =>
                checkpoint.account ? go('protect') : void createAccount()
              }
            >
              {checkpoint.account
                ? 'Continue'
                : signupPending
                  ? 'Resume account setup'
                  : 'Create my account'}
            </Button>
          )}
        </Foot>
      }
    >
      <h1>Set up your account</h1>
      <p className="lead">
        {signingIn ? (
          <>
            Your account already exists on {profile?.canonicalName}. Recover it
            with your recovery phrase
            {goCandidatePresent
              ? ' or connect using the official FOKS CLI'
              : ''}
            .
          </>
        ) : (
          'Your account keys are generated on this device; only the public keys are sent to the server.'
        )}
      </p>
      <StepLabel n={1}>Setup method</StepLabel>
      <Inset>
        <RadioGroup label="Account setup">
          <div className="choice">
            <RadioCard
              title="Create a new account"
              detail="Set up a new FOKS account on this device."
              selected={!signingIn && !ssoSelected}
              disabled={Boolean(checkpoint.sso) || cliAccountSelected}
              off={cliAccountSelected}
              onSelect={() => {
                go('account');
              }}
            />
            {!signingIn && !ssoSelected ? (
              <div className="choice-body">
                <Toggle
                  label="Email or invite code"
                  defaultOpen={Boolean(email || invite)}
                >
                  <Inset className="account-form">
                    <InsetRow label="Email">
                      <input
                        value={email}
                        placeholder="you@example.net"
                        onChange={(event) => setEmail(event.target.value)}
                      />
                    </InsetRow>
                    <InsetRow label="Invite code">
                      <input
                        value={invite}
                        onChange={(event) => setInvite(event.target.value)}
                      />
                    </InsetRow>
                  </Inset>
                </Toggle>
              </div>
            ) : null}
          </div>
          {ssoAvailable && profile ? (
            <div className="choice">
              <RadioCard
                title="Sign up with SSO"
                detail="Create the account through your organization’s identity provider."
                selected={ssoSelected}
                disabled={cliAccountSelected}
                off={cliAccountSelected}
                onSelect={() => {
                  onChooseOrganization();
                }}
              />
              {ssoSelected ? (
                <div className="choice-body">
                  <Toggle label="Invite code" defaultOpen={Boolean(invite)}>
                    <Inset className="account-form">
                      <InsetRow label="Invite code">
                        <input
                          value={invite}
                          onChange={(event) => setInvite(event.target.value)}
                        />
                      </InsetRow>
                    </Inset>
                  </Toggle>
                  {organizationSignup}
                </div>
              ) : null}
            </div>
          ) : null}
          <div className="choice">
            <RadioCard
              title="Sign in to an existing account"
              detail="Add this device to an account you already have."
              selected={signingIn}
              onSelect={() => {
                if (signingIn) return;
                openExisting();
              }}
            />
          </div>
        </RadioGroup>
      </Inset>
      {signingIn ? (
        signinSections(2)
      ) : (
        <>
          <StepLabel n={2}>Account and device</StepLabel>
          <Inset className="account-form">{identityRows}</Inset>
          {aliasInvalidNotice}
          {message ? (
            <p className="crit" role="alert">
              {failureText}
            </p>
          ) : null}
          {duplicateAlias && !busy && !identityLoading ? (
            <div className="band info" role="status">
              <span className="t">
                “{duplicateAlias.alias}” is already set up on this device for
                this server. Use that account, or choose a different username.
              </span>
              <span className="a">
                <Button
                  variant="primary"
                  onClick={() => void adoptDuplicateAccount()}
                >
                  Use existing account
                </Button>
              </span>
            </div>
          ) : null}
        </>
      )}
    </Pane>
  );
}
