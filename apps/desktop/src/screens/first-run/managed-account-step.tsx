import type { ReactNode } from 'react';
import { Button } from '../../components';
import { fixDeviceName } from '../../device-name';
import { Foot, Pane } from '../first-run-view';
import { FailureText } from './failure-text';
import { useSetupContext } from './setup-context';

/** Create your account on the managed local server: a username and a device name. */
export function ManagedAccountForm(): ReactNode {
  const {
    checkpoint,
    go,
    busy: busyWith,
    message,
    reasonFor,
    pending,
    identity,
    secrets,
    account,
  } = useSetupContext();
  const busy = busyWith !== null;
  const { username, deviceName, email, accountAlias, aliasInvalid } = identity;
  const { invite, setInvite } = secrets;
  const { duplicateAlias, identityLoading } = account;
  return (
    <Pane
      title="Your account"
      header={false}
      foot={
        <Foot back={() => go('local')}>
          <Button
            variant="primary"
            disabled={
              busy ||
              aliasInvalid ||
              (!checkpoint.account &&
                (!username.trim() ||
                  !fixDeviceName(deviceName) ||
                  !accountAlias))
            }
            onClick={() =>
              checkpoint.account ? go('protect') : void account.create()
            }
          >
            {checkpoint.account
              ? 'Continue'
              : pending.some(
                    (row) =>
                      row.kind === 'account-signup' &&
                      row.alias === accountAlias,
                  )
                ? 'Resume account setup'
                : 'Create my account'}
          </Button>
        </Foot>
      }
    >
      <h1>Create your account</h1>
      <p className="lead">
        Choose a username for this server and a name for this device.
      </p>
      <div className="local-field-card">
        <label className="local-field-row">
          <span>Username</span>
          <input
            value={username}
            placeholder="yourname"
            autoFocus
            disabled={Boolean(checkpoint.account)}
            onChange={(event) => identity.edit(event.target.value)}
          />
        </label>
        <label className="local-field-row">
          <span>Device name</span>
          <input
            value={deviceName}
            placeholder="Your device"
            disabled={Boolean(checkpoint.account)}
            onChange={(event) => identity.setDeviceName(event.target.value)}
            onBlur={() => identity.setDeviceName(fixDeviceName(deviceName))}
          />
        </label>
      </div>
      <details className="local-more">
        <summary>More options</summary>
        <label className="local-field-row">
          <span>Email (optional)</span>
          <input
            value={email}
            disabled={Boolean(checkpoint.account)}
            placeholder="you@example.net"
            onChange={(event) => identity.setEmail(event.target.value)}
          />
        </label>
        <label className="local-field-row">
          <span>Invite code (optional)</span>
          <input
            value={invite}
            disabled={Boolean(checkpoint.account)}
            onChange={(event) => setInvite(event.target.value)}
          />
        </label>
      </details>
      {aliasInvalid ? (
        <p className="crit" role="alert">
          Username must contain at least one letter or number.
        </p>
      ) : null}
      {message ? (
        <p className="crit" role="alert">
          <FailureText text={message} reason={reasonFor(message)} />
        </p>
      ) : null}
      {duplicateAlias && !busy && !identityLoading ? (
        <div className="band info" role="status">
          <span className="t">
            “{duplicateAlias.alias}” is already set up on this device for this
            server. Use that account, or choose a different username.
          </span>
          <span className="a">
            <Button
              variant="primary"
              onClick={() => void account.adoptDuplicateAccount()}
            >
              Use existing account
            </Button>
          </span>
        </div>
      ) : null}
      <button className="lnk local-recover-link" onClick={() => go('existing')}>
        Recover an existing account…
      </button>
    </Pane>
  );
}
