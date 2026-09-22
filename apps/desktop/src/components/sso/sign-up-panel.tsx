/** SSO sign-up panel used by the first-run account form. */

import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { Bridge } from '../../bridge';
import type { SsoAction, SsoProgress } from '../../sso-contract';
import { Button } from '../button';
import { Inset, InsetRow } from '../inset';
import { RadioCard, RadioGroup } from '../radio-card';
import { SectionLabel } from '../section-label';
import { SsoOutcome, SsoStatus } from './sso-status';
import { useSsoFlow } from './use-sso-flow';

export interface SsoSignUpPanelProps {
  bridge: Bridge;
  profile: string;
  account: string;
  deviceName?: string;
  invite?: string;
  disabled?: boolean;
  onComplete: () => void | Promise<void>;
  initialOperationId?: string;
  initialHardware?: boolean;
  onProgress?: (progress: SsoProgress, hardware: boolean) => void;
  executeSignup?: (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ) => Promise<SsoProgress>;
  /** Restrict the panel to reading and completing a saved sign-up. */
  resumeOnly?: boolean;
  /** Omit the standalone card border and heading. */
  embedded?: boolean;
  /** Optional portal target for the primary action. */
  primarySlot?: HTMLElement | null;
}

export function SsoSignUpPanel({
  bridge,
  profile,
  account,
  deviceName = '',
  invite = '',
  disabled = false,
  onComplete,
  initialOperationId,
  initialHardware = false,
  onProgress,
  executeSignup,
  resumeOnly = false,
  embedded = false,
  primarySlot = null,
}: SsoSignUpPanelProps): ReactNode {
  const flow = useSsoFlow({
    bridge,
    profile,
    account,
    login: false,
    deviceName,
    invite,
    disabled,
    onComplete,
    initialOperationId,
    initialHardware,
    onProgress,
    executeSignup,
    resumeOnly,
  });
  const { progress, phase, blocked, error } = flow;
  const errorLine = error ? (
    <p role="alert" className="crit">
      {error}
    </p>
  ) : null;

  let body: ReactNode;
  let primaryButton: ReactNode = null;
  let secondary: ReactNode = null;
  if (phase === 'done' && progress) {
    body = (
      <>
        <SsoOutcome progress={progress} />
        {progress.state === 'service-unavailable' ? (
          <div className="btns">
            <Button disabled={blocked} onClick={flow.status}>
              Refresh status
            </Button>
          </div>
        ) : null}
        {errorLine}
      </>
    );
  } else if (phase !== 'start' && progress) {
    const primary: 'open' | 'check' | 'finish' = flow.finishable
      ? 'finish'
      : flow.browserAvailable && !flow.opened
        ? 'open'
        : 'check';
    const openButton = flow.browserAvailable ? (
      <Button
        variant={primary === 'open' ? 'primary' : 'plain'}
        disabled={blocked}
        onClick={flow.openBrowser}
      >
        Open sign-in browser
      </Button>
    ) : null;
    const checkButton = flow.pollable ? (
      <Button
        variant={primary === 'check' ? 'primary' : 'plain'}
        disabled={blocked}
        onClick={flow.poll}
      >
        Check sign-in
      </Button>
    ) : null;
    const finishButton = flow.finishable ? (
      <Button
        variant="primary"
        title={flow.eligibility.title}
        disabled={blocked || (flow.hardwareRequired && !flow.pin)}
        onClick={flow.finish}
      >
        Finish sign-up
      </Button>
    ) : null;
    primaryButton =
      primary === 'finish'
        ? finishButton
        : primary === 'open'
          ? openButton
          : checkButton;
    secondary = (
      <>
        {primary === 'open' ? null : openButton}
        {primary === 'check' ? null : checkButton}
        {flow.cancellable ? (
          <Button disabled={blocked} onClick={flow.cancel}>
            Cancel sign-in
          </Button>
        ) : null}
        <Button disabled={blocked} onClick={flow.status}>
          Refresh status
        </Button>
      </>
    );
    body = (
      <>
        <SsoStatus progress={progress} />
        {flow.hardwareRequired ? (
          <Inset className="account-form">
            <InsetRow label="Security key PIN">
              <input
                type="password"
                autoComplete="off"
                aria-label="Security key PIN"
                value={flow.pin}
                disabled={blocked}
                onChange={(event) => flow.setPin(event.target.value)}
              />
            </InsetRow>
          </Inset>
        ) : null}
        {errorLine}
      </>
    );
  } else {
    primaryButton = resumeOnly ? null : (
      <Button
        variant="primary"
        title={flow.eligibility.title}
        disabled={flow.beginDisabled}
        onClick={flow.begin}
      >
        {progress ? 'Try again' : 'Continue with organization'}
      </Button>
    );
    body = (
      <>
        <p>
          {resumeOnly
            ? 'A sign-up with your organization is saved on this device. Check its status to finish it.'
            : 'Your identity provider supplies your username and email. The username above is this device’s label for the account.'}
        </p>
        {flow.eligibility.title ? (
          <p role="status">{flow.eligibility.title}</p>
        ) : null}
        {progress ? <SsoStatus progress={progress} /> : null}
        {errorLine}
        {resumeOnly ? null : (
          <>
            <SectionLabel>Keys</SectionLabel>
            <Inset>
              <RadioGroup label="Where to keep keys">
                <RadioCard
                  selected={!flow.hardware}
                  disabled={blocked}
                  onSelect={() => flow.chooseHardware(false)}
                  title="This computer"
                  detail="Keys stay in this device’s secure storage."
                />
                <RadioCard
                  selected={flow.hardware}
                  disabled={blocked}
                  onSelect={() => flow.chooseHardware(true)}
                  title="Security key"
                  detail="Keys are created on a plugged-in security key, in signing slot 130 and encryption slot 131."
                />
              </RadioGroup>
            </Inset>
            {flow.hardware ? (
              <>
                <Inset className="account-form">
                  <InsetRow label="Card serial">
                    <input
                      value={flow.serial}
                      inputMode="numeric"
                      aria-label="Card serial"
                      placeholder="Printed on the key"
                      disabled={blocked}
                      onChange={(event) => flow.setSerial(event.target.value)}
                    />
                  </InsetRow>
                  <InsetRow label="Security key PIN">
                    <input
                      type="password"
                      autoComplete="off"
                      aria-label="Security key PIN"
                      value={flow.pin}
                      disabled={blocked}
                      onChange={(event) => flow.setPin(event.target.value)}
                    />
                  </InsetRow>
                </Inset>
                <p>
                  Existing keys in slots 130 and 131 are checked before anything
                  is written.
                </p>
              </>
            ) : null}
          </>
        )}
      </>
    );
  }

  // Omit the local action row when its primary action is portaled and there
  // are no secondary actions.
  const showActions =
    Boolean(secondary) || Boolean(!primarySlot && primaryButton);
  return (
    <section
      className={embedded ? 'sso-inline' : 'pcard'}
      aria-label="Organization sign-in"
    >
      {embedded ? null : <h3>Sign up with your organization</h3>}
      {body}
      {primarySlot ? createPortal(primaryButton, primarySlot) : null}
      {showActions ? (
        <div className="btns">
          {primarySlot ? null : primaryButton}
          {secondary}
        </div>
      ) : null}
    </section>
  );
}
