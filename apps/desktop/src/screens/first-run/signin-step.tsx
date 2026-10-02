import type { ReactNode } from 'react';
import {
  Button,
  CopyBox,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
} from '../../components';
import { fixDeviceName } from '../../device-name';
import { pastePhrase } from '../../phrase-input';
import type { SigninMethod } from './session-transitions';
import { StepLabel } from './step-label';

export function SigninPrimaryAction({
  signinMethod,
  busy,
  accountAlias,
  recoveryPhrase,
  pairingPhrase,
  deviceName,
  recoveryPending,
  recover,
  copyGoCandidate,
  acceptPairing,
}: {
  signinMethod: SigninMethod | null;
  busy: boolean;
  accountAlias: string;
  recoveryPhrase: string;
  pairingPhrase: string;
  deviceName: string;
  recoveryPending: boolean;
  recover: () => Promise<void>;
  copyGoCandidate: () => Promise<void>;
  acceptPairing: (resume: boolean) => Promise<void>;
}): ReactNode {
  return signinMethod === 'recover' ? (
    <Button
      variant="primary"
      disabled={
        busy ||
        !accountAlias ||
        !recoveryPhrase.trim() ||
        !fixDeviceName(deviceName)
      }
      onClick={() => void recover()}
    >
      {recoveryPending ? 'Resume recovery' : 'Recover'}
    </Button>
  ) : signinMethod === 'import' ? (
    <Button
      variant="primary"
      disabled={busy || !accountAlias}
      onClick={() => void copyGoCandidate()}
    >
      Import credentials
    </Button>
  ) : signinMethod === 'pair' ? (
    <Button
      variant="primary"
      disabled={
        busy ||
        !accountAlias ||
        !fixDeviceName(deviceName) ||
        !pairingPhrase.trim()
      }
      onClick={() => void acceptPairing(false)}
    >
      Accept pairing
    </Button>
  ) : (
    <Button variant="primary" disabled>
      Continue
    </Button>
  );
}

/** Pairing, credential import, and recovery share identity fields but retain distinct actions. */
export function SigninSections({
  first,
  signinMethod,
  pairable,
  copyable,
  accountConnected,
  identityRows,
  aliasInvalidNotice,
  recoveryPhrase,
  pairingPhrase,
  busy,
  accountAlias,
  deviceName,
  setChosenSigninMethod,
  setRecoveryPhrase,
  setPairingPhrase,
  acceptPairing,
  onCopyCommand,
  error,
}: {
  first: number;
  signinMethod: SigninMethod | null;
  pairable: boolean;
  copyable: boolean;
  accountConnected: boolean;
  identityRows: ReactNode;
  aliasInvalidNotice: ReactNode;
  recoveryPhrase: string;
  pairingPhrase: string;
  busy: boolean;
  accountAlias: string;
  deviceName: string;
  setChosenSigninMethod: (method: SigninMethod) => void;
  setRecoveryPhrase: (phrase: string) => void;
  setPairingPhrase: (phrase: string) => void;
  acceptPairing: (resume: boolean) => Promise<void>;
  onCopyCommand: (value: string) => void;
  error: ReactNode;
}): ReactNode {
  return (
    <>
      <StepLabel n={first}>Sign in method</StepLabel>
      <Inset>
        <RadioGroup label="Sign-in method">
          {pairable ? (
            <RadioCard
              title="Use the FOKS CLI to link this as a new device"
              detail={
                <>
                  Pair the desktop app with{' '}
                  <strong>foks --simple-ui key assist</strong> in Terminal.
                </>
              }
              selected={signinMethod === 'pair'}
              onSelect={() => setChosenSigninMethod('pair')}
            />
          ) : null}
          {copyable ? (
            <RadioCard
              title="Import this device’s FOKS CLI credentials"
              detail="Copy your device credentials from the FOKS CLI. This may require a Keychain prompt."
              selected={signinMethod === 'import'}
              onSelect={() => setChosenSigninMethod('import')}
            />
          ) : null}
          <RadioCard
            title="Recover with recovery phrase"
            detail="Enter your recovery phrase to restore full access on this device."
            selected={signinMethod === 'recover'}
            onSelect={() => setChosenSigninMethod('recover')}
          />
        </RadioGroup>
      </Inset>
      {/* If the account already exists, display its details as read-only fields
          regardless of the selected sign-in method. */}
      {signinMethod || accountConnected ? (
        <>
          <StepLabel n={first + 1}>Account and device</StepLabel>
          {signinMethod === 'pair' ? (
            <>
              <p className="hint">
                In Terminal, switch the official FOKS CLI to this account, then
                run:
              </p>
              <CopyBox
                text="foks --simple-ui key assist"
                onCopy={(value) => onCopyCommand(value)}
              >
                <code>foks --simple-ui key assist</code>
              </CopyBox>
            </>
          ) : null}
          <Inset className="account-form">
            {identityRows}
            {signinMethod === 'recover' ? (
              <InsetRow label="Phrase">
                <input
                  type="password"
                  aria-label="Recovery phrase"
                  value={recoveryPhrase}
                  onChange={(event) => setRecoveryPhrase(event.target.value)}
                  onPaste={(event) => pastePhrase(event, setRecoveryPhrase)}
                />
              </InsetRow>
            ) : null}
            {signinMethod === 'pair' ? (
              <InsetRow label="Pairing phrase">
                <input
                  type="password"
                  aria-label="Pairing phrase"
                  placeholder="Enter pairing phrase"
                  value={pairingPhrase}
                  onChange={(event) => setPairingPhrase(event.target.value)}
                  onPaste={(event) => pastePhrase(event, setPairingPhrase)}
                />
              </InsetRow>
            ) : null}
          </Inset>
          {aliasInvalidNotice}
          {signinMethod === 'import' ? (
            <p className="hint">
              Both apps will share the same device credentials. Revoking the
              device in either client will disable both.
            </p>
          ) : null}
          {signinMethod === 'pair' ? (
            <p className="hint">
              Select the account in the CLI, enter the pairing phrase above, and
              follow the terminal prompts to complete pairing. Or,{' '}
              <button
                type="button"
                className="lnk"
                aria-label="Resume pairing"
                disabled={busy || !accountAlias || !fixDeviceName(deviceName)}
                onClick={() => void acceptPairing(true)}
              >
                resume pairing
              </button>{' '}
              for an existing account.
            </p>
          ) : null}
          {error}
        </>
      ) : null}
    </>
  );
}
