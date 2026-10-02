import type { ReactNode, RefObject } from 'react';
import { Button, CopyBox, Inset, Toggle } from '../../components';
import type { FirstRunPath, FirstRunStateName } from '../../first-run-state';
import type { presentFirstRunFailure } from '../../first-run-failure';
import { Foot, Pane } from '../first-run-view';

/** Address entry and failure presentation; server verification is a separate workflow. */
export function ServerAddressStep({
  state,
  path,
  busy,
  adminShort,
  address,
  addressInvalid,
  inputRef,
  serverCheckPresentation,
  message,
  go,
  checkServer,
  editServerAddress,
  onCopy,
}: {
  state: FirstRunStateName;
  path: FirstRunPath;
  busy: boolean;
  adminShort: string;
  address: string;
  addressInvalid: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  serverCheckPresentation: ReturnType<typeof presentFirstRunFailure> | null;
  message: string | null;
  go: (state: FirstRunStateName) => void;
  checkServer: () => Promise<void>;
  editServerAddress: (address: string) => void;
  onCopy: (text: string) => void;
}): ReactNode {
  return (
    <Pane
      title={path === 'invited' ? 'Team Server' : 'Server Setup'}
      header={false}
      scope={
        state === 'no-address' && path === 'invited'
          ? 'Finding your server address'
          : undefined
      }
      foot={
        <Foot back={() => go('who')}>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => void checkServer()}
          >
            Continue
          </Button>
        </Foot>
      }
    >
      <h1>
        {path === 'invited' ? 'Select a server address' : 'Select a server'}
      </h1>
      <p className="lead">
        FOKS synchronizes your account, teams, and encrypted vaults through a
        server.{' '}
        {path === 'invited'
          ? `Enter the server address provided by ${adminShort}.`
          : null}
      </p>
      <Inset
        className={state === 'error' || addressInvalid ? 'err' : undefined}
      >
        <label className="fr server-address-row">
          <span className="k">Address</span>
          <span className="v">
            <input
              ref={inputRef}
              aria-label="Server address"
              value={address}
              placeholder="e.g. foks.app:4430"
              onChange={(event) => {
                editServerAddress(event.target.value);
              }}
            />
          </span>
        </label>
      </Inset>
      <Button
        className="official-server"
        disabled={busy}
        onClick={() => {
          editServerAddress('foks.app:4430');
        }}
      >
        Use the official FOKS server
      </Button>
      {state === 'error' && !addressInvalid ? (
        <div className="crit" role="alert">
          <b>
            {serverCheckPresentation?.title ??
              message ??
              (address
                ? `Could not connect to ${address}`
                : 'No server address provided')}
          </b>
          <p>
            {serverCheckPresentation?.detail ??
              'Confirm the address is correct, then retry.'}
          </p>
          {serverCheckPresentation?.reason ? (
            <Toggle label="Details">
              <pre>{serverCheckPresentation.reason}</pre>
            </Toggle>
          ) : null}
        </div>
      ) : null}
      {path === 'invited' && state === 'no-address' ? (
        <div className="pcard">
          <h3>{`Ask ${adminShort} for the server address`}</h3>
          <p>
            Contact them through your usual communication channel. This is the
            only detail needed right now.
          </p>
          <CopyBox
            text="What’s the address of the FOKS server our team is on?"
            onCopy={(value) => onCopy(value)}
          >
            “What’s the address of the FOKS server our team is on?”
          </CopyBox>
          <p>
            In the next step, you will choose a username and share it with them
            so they can add you to the team.
          </p>
        </div>
      ) : path === 'invited' ? (
        <p className="hint">
          <button className="lnk" onClick={() => go('no-address')}>
            Don’t have a server address?
          </button>
        </p>
      ) : null}
    </Pane>
  );
}
