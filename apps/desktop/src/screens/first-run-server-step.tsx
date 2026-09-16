import type { ReactNode, RefObject } from 'react';
import { Button, Icon, Inset, InsetRow, Toggle } from '../components';
import type { FirstRunCheckpoint, FirstRunStateName } from '../first-run-state';
import type { CheckedProfileResponse } from '../bridge';
import { Pane, Foot } from './first-run-view';
interface Props {
  checkpoint: FirstRunCheckpoint;
  state: FirstRunStateName;
  profile: CheckedProfileResponse;
  address: string;
  busy: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  onBack: () => void;
  onContinue: () => void;
  onCheck: () => void;
  onAddressChange: (value: string) => void;
}
export function ServerVerificationStep({
  checkpoint,
  state,
  profile,
  address,
  busy,
  inputRef,
  onBack,
  onContinue,
  onCheck,
  onAddressChange,
}: Props): ReactNode {
  return (
    <Pane
      title={checkpoint.path === 'invited' ? 'Team server' : 'Server details'}
      header={false}
      scope="Server verified and pinned. No user data sent."
      foot={
        <Foot back={onBack}>
          <Button variant="primary" disabled={!profile} onClick={onContinue}>
            Continue
          </Button>
        </Foot>
      }
    >
      <h1>Select a server</h1>
      <p className="lead">
        FOKS synchronizes your account, teams, and encrypted vaults through a
        server.
      </p>
      <Inset className="checked-address">
        <InsetRow
          label="Address"
          action={
            <Button disabled={busy} onClick={onCheck}>
              Check again
            </Button>
          }
        >
          <input
            value={address}
            placeholder="e.g. foks.app:4430"
            spellCheck={false}
            ref={inputRef}
            aria-label="Server address"
            onChange={(event) => onAddressChange(event.target.value)}
          />
        </InsetRow>
      </Inset>
      <div className="pcard ok">
        <h3>
          <Icon name="server" /> {profile?.canonicalName} verified{' '}
        </h3>
        <p>
          The server certificate was verified on first connection, and its host
          ID is now pinned for future connections.
        </p>
        <Toggle label="Details" defaultOpen={state === 'compare'}>
          <div className="dbody">
            <div className="facts">
              <div>
                <span className="k">Lookup name</span>
                <code>{profile?.lookupName}</code>
              </div>
              <div>
                <span className="k">Confirmed name</span>
                <code>{profile?.canonicalName}</code>
              </div>
              <div>
                <span className="k">Server version</span>
                <span>{profile?.chain}</span>
              </div>
              <div>
                <span className="k">Verified</span>
                <span>
                  {profile
                    ? new Date(profile.epoch * 1000).toLocaleDateString()
                    : '—'}
                </span>
              </div>
            </div>
            <Toggle label="Inspect response">
              <pre>{JSON.stringify(profile, null, 1)}</pre>
            </Toggle>
          </div>
        </Toggle>
      </div>
    </Pane>
  );
}
