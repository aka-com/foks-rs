import type { ReactNode } from 'react';
import { Button, InsetRow } from '../../components';
import { SsoPanel } from '../settings';
import { fixDeviceName } from '../../device-name';
import type { PendingOperation } from '../../bridge';
import { updateRetainedSetup } from './first-run-recovery';
import { useToast } from '../../../kit/toasts';
import { Foot, Pane } from './first-run-view';
import { AccountSetupStep } from './account-setup-step';
import { FailureText } from './failure-text';
import { ManagedAccountForm } from './managed-account-step';
import { selectedSigninMethod } from './session-transitions';
import { useSetupContext } from './setup-context';
import { SigninPrimaryAction, SigninSections } from './signin-step';

/**
 * The account steps. `account` and `existing` render one page with the
 * create and sign-in choices, so a move between them keeps it mounted; the
 * managed local server has its own form and its own sign-in page instead.
 */
export function AccountStep(): ReactNode {
  const { checkpoint } = useSetupContext();
  if (!checkpoint.managedLocal) return <AccountChoice />;
  return checkpoint.state === 'account' ? (
    <ManagedAccountForm />
  ) : (
    <ManagedSignin />
  );
}

/** Where the account page's Back goes; signing in shares it. */
function accountBackTarget(managedLocal: boolean): 'local' | 'checked' {
  return managedLocal ? 'local' : 'checked';
}

/**
 * The parts both sign-in pages draw: the account identity rows, the chosen
 * method's sections and the primary action for that method.
 */
function useSigninParts() {
  const toasts = useToast();
  const {
    bridge,
    busy: busyWith,
    checkpoint,
    pending,
    reasonFor,
    identity,
    secrets,
    cli,
    account,
    signin,
  } = useSetupContext();
  const busy = busyWith !== null;
  const { username, deviceName, accountAlias, aliasInvalid } = identity;
  const { recoveryPhrase, pairingPhrase } = secrets;
  const goCandidate = cli.candidate;
  /* The account alias and this device’s name identify the same account
     whichever way it is reached, so both account pages draw the same two
     rows. On the sign-in path the chosen method's own row joins them. */
  const identityRows = (
    <>
      <InsetRow label="Your name">
        <input
          aria-label="Your name"
          value={checkpoint.account?.username ?? username}
          placeholder="yourname"
          disabled={Boolean(checkpoint.account)}
          onChange={(event) => identity.edit(event.target.value)}
        />
      </InsetRow>
      <InsetRow label="This device’s name">
        <input
          value={checkpoint.account?.deviceName ?? deviceName}
          placeholder="Your device"
          disabled={Boolean(checkpoint.account)}
          onChange={(event) => identity.setDeviceName(event.target.value)}
          onBlur={() => identity.setDeviceName(fixDeviceName(deviceName))}
        />
      </InsetRow>
    </>
  );
  const aliasInvalidNotice = aliasInvalid ? (
    <p className="crit" role="alert">
      Your name must contain at least one letter or number.
    </p>
  ) : null;
  /* Sign-in authentication methods: CLI pairing and credential import when an
     eligible FOKS CLI profile is detected, followed by recovery phrase recovery.
     When only one method is available, it is selected automatically. */
  const pendingFor = (kind: PendingOperation['kind']): boolean =>
    pending.some((row) => row.kind === kind && row.alias === accountAlias);
  const signinMethod = selectedSigninMethod({
    candidate: goCandidate,
    chosen: signin.chosen,
    pending,
    alias: accountAlias,
  });
  const signinPrimary = (
    <SigninPrimaryAction
      signinMethod={signinMethod}
      busy={busy}
      accountAlias={accountAlias}
      recoveryPhrase={recoveryPhrase}
      pairingPhrase={pairingPhrase}
      deviceName={deviceName}
      recoveryPending={pendingFor('account-recovery')}
      recover={account.recover}
      copyGoCandidate={account.copyCandidate}
      acceptPairing={account.acceptPairing}
    />
  );
  const methodError = (
    key: keyof typeof account.connectionErrors,
  ): ReactNode => {
    const text = account.connectionErrors[key];
    return text ? (
      <p className="crit" role="alert">
        <FailureText text={text} reason={reasonFor(text)} />
      </p>
    ) : null;
  };
  const signinSections = (first: number): ReactNode => (
    <SigninSections
      first={first}
      signinMethod={signinMethod}
      pairable={Boolean(goCandidate?.pairable)}
      copyable={Boolean(goCandidate?.copyable)}
      accountConnected={Boolean(checkpoint.account)}
      identityRows={identityRows}
      aliasInvalidNotice={aliasInvalidNotice}
      recoveryPhrase={recoveryPhrase}
      pairingPhrase={pairingPhrase}
      busy={busy}
      accountAlias={accountAlias}
      deviceName={deviceName}
      setChosenSigninMethod={signin.setChosen}
      setRecoveryPhrase={secrets.setRecoveryPhrase}
      setPairingPhrase={secrets.setPairingPhrase}
      acceptPairing={account.acceptPairing}
      onCopyCommand={(value) =>
        void bridge.copyText(value).then(() => toasts.show('Command copied.'))
      }
      error={
        signinMethod
          ? methodError(signinMethod === 'import' ? 'copy' : signinMethod)
          : null
      }
    />
  );
  return { identityRows, aliasInvalidNotice, signinPrimary, signinSections };
}

/** Create an account, sign in to an existing one, or sign up through an organization. */
function AccountChoice(): ReactNode {
  const {
    bridge,
    checkpoint,
    checkpointRef,
    commit,
    send,
    go,
    busy: busyWith,
    message,
    reasonFor,
    pending,
    identity,
    secrets,
    cli,
    account,
    sso,
  } = useSetupContext();
  const parts = useSigninParts();
  const busy = busyWith !== null;
  const { username, deviceName, accountAlias } = identity;
  const { invite, setInvite } = secrets;
  const profile = checkpoint.profile;
  // Both choices render on this one page. `existing` is the same page with
  // "Sign in to an existing account" selected and the sign-in method group
  // drawn as the next section.
  const signingIn = checkpoint.state === 'existing';
  const cliAccountSelected = signingIn && cli.candidateExplicit.current;
  // Organization sign-up is a third choice in the same radio group. It is
  // offered only while the server is known and no account exists yet, and
  // its own panel carries the primary action.
  const ssoAvailable = Boolean(profile) && !checkpoint.account;
  const ssoSelected = !signingIn && ssoAvailable && sso.shown;
  const organizationSignup = profile ? (
    <SsoPanel
      key={`${profile.profile}/${accountAlias}`}
      embedded
      primarySlot={sso.primarySlot}
      bridge={bridge}
      profile={profile.profile}
      account={accountAlias}
      login={false}
      deviceName={fixDeviceName(deviceName)}
      invite={invite}
      disabled={busy}
      initialOperationId={
        checkpoint.sso?.alias === accountAlias
          ? checkpoint.sso.operationId
          : undefined
      }
      initialHardware={checkpoint.sso?.hardware}
      executeSignup={account.executeSsoSignup}
      onProgress={(progress, hardware) => {
        if (
          ['cancelled', 'expired', 'denied', 'rejected'].includes(
            progress.state,
          ) &&
          !checkpointRef.current.provisioning
        ) {
          const next = {
            ...checkpointRef.current,
            sso: undefined,
            accountMethod: 'create' as const,
          };
          updateRetainedSetup(checkpointRef.current, next);
          commit(next);
          return;
        }
        if (
          progress.operationId &&
          !checkpointRef.current.provisioning &&
          !checkpointRef.current.provisionedAccount
        )
          commit({
            ...checkpointRef.current,
            sso: {
              operationId: progress.operationId,
              alias: accountAlias,
              hardware,
            },
          });
      }}
      onComplete={() => {
        setInvite('');
        account.accountProvisioned(accountAlias, fixDeviceName(deviceName));
      }}
    />
  ) : null;
  return (
    <AccountSetupStep
      checkpoint={checkpoint}
      signingIn={signingIn}
      cliAccountSelected={cliAccountSelected}
      ssoAvailable={ssoAvailable}
      ssoSelected={ssoSelected}
      busy={busy}
      usernameAliasInvalid={identity.aliasInvalid}
      username={username}
      deviceName={deviceName}
      signupPending={pending.some(
        (row) => row.kind === 'account-signup' && row.alias === accountAlias,
      )}
      email={identity.email}
      invite={invite}
      goCandidatePresent={Boolean(cli.candidate)}
      accountBackTarget={accountBackTarget(checkpoint.managedLocal)}
      identityLoading={account.identityLoading}
      duplicateAlias={account.duplicateAlias}
      message={message}
      identityRows={parts.identityRows}
      aliasInvalidNotice={parts.aliasInvalidNotice}
      signinPrimary={parts.signinPrimary}
      signinSections={parts.signinSections}
      organizationSignup={organizationSignup}
      failureText={
        <FailureText text={message ?? ''} reason={reasonFor(message)} />
      }
      go={go}
      createAccount={account.create}
      openExisting={() => go('existing')}
      setEmail={identity.setEmail}
      setInvite={setInvite}
      onPrimarySlot={sso.setPrimarySlot}
      onChooseOrganization={() => {
        secrets.clear();
        send({ type: 'select-account-method', method: 'organization' });
      }}
      adoptDuplicateAccount={account.adoptDuplicateAccount}
    />
  );
}

/**
 * Only the managed-local path keeps a separate page for an existing account;
 * its Create your account pane has no setup-method radios, so the sign-in
 * method is this page's first section.
 */
function ManagedSignin(): ReactNode {
  const { checkpoint, go, cli } = useSetupContext();
  const parts = useSigninParts();
  return (
    <Pane
      title="Your account"
      header={false}
      scope="Device authorization required"
      wide
      foot={
        <Foot
          back={() =>
            go(
              checkpoint.account
                ? 'protect'
                : accountBackTarget(checkpoint.managedLocal),
            )
          }
        >
          {checkpoint.account ? (
            <Button onClick={() => go('protect')}>Skip to latest step</Button>
          ) : (
            <>
              <Button
                onClick={() => {
                  go('account');
                }}
              >
                Create a new account
              </Button>
              {parts.signinPrimary}
            </>
          )}
        </Foot>
      }
    >
      <h1>Add this device to your account</h1>
      <p className="lead">
        Your account already exists on {checkpoint.profile?.canonicalName}.
        Recover it with your recovery phrase
        {cli.candidate ? ' or connect using the official FOKS CLI' : ''}.
      </p>
      {parts.signinSections(1)}
    </Pane>
  );
}
