/** Two-step SSO sheet for reauthenticating or linking an existing account. */

import type { ReactNode } from 'react';
import type { Bridge } from '../../bridge';
import { Button } from '../button';
import { Icon } from '../icon';
import { Inset, InsetRow } from '../inset';
import { PanelSheet } from '../panel-sheet';
import type { PanelPresentation } from '../panel-sheet';
import { SsoOutcome, SsoStateChip, SsoStatus } from './sso-status';
import { useSsoFlow } from './use-sso-flow';

export function SsoSignInSheet({
  bridge,
  profile,
  account,
  presentation,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  presentation: PanelPresentation;
  onComplete: () => void | Promise<void>;
}): ReactNode {
  const flow = useSsoFlow({
    bridge,
    profile,
    account,
    login: true,
    onComplete,
  });
  const { progress, phase, busy, blocked, error } = flow;
  const linkage = progress?.accountStatus ? progress : null;
  const errorLine = error ? (
    <p role="alert" className="crit">
      {error}
    </p>
  ) : null;
  const pinRow = (
    <InsetRow label="Security key PIN">
      <input
        type="password"
        autoComplete="off"
        placeholder="Unlocks the enrolled key"
        maxLength={32}
        value={flow.pin}
        disabled={blocked}
        onChange={(event) => flow.setPin(event.target.value)}
      />
    </InsetRow>
  );
  const glyph = (
    <span className="kico invite">
      <Icon name="globe" />
    </span>
  );

  if (phase === 'done' && progress)
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        glyph={glyph}
        title={progress.serviceAccess ? 'Signed in' : 'Sign-in accepted'}
        footer={
          <>
            {progress.state === 'service-unavailable' ? (
              <Button disabled={blocked} onClick={flow.status}>
                Refresh status
              </Button>
            ) : null}
            <span className="spacer" />
            <Button
              variant="primary"
              disabled={busy}
              onClick={presentation.onClose}
            >
              Done
            </Button>
          </>
        }
      >
        <SsoOutcome progress={progress} />
        {errorLine}
      </PanelSheet>
    );

  if (phase !== 'start' && progress) {
    const verified = phase === 'verified';
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
        Finish sign-in
      </Button>
    ) : null;
    // Render the next required action as the final primary button.
    const primaryButton =
      primary === 'finish'
        ? finishButton
        : primary === 'open'
          ? openButton
          : checkButton;
    return (
      <PanelSheet
        presentation={presentation}
        busy={busy}
        glyph={glyph}
        step="Step 2 of 2"
        title={verified ? 'Browser sign-in verified' : 'Finish in your browser'}
        footer={
          <>
            {flow.cancellable ? (
              <Button disabled={blocked} onClick={flow.cancel}>
                Cancel sign-in
              </Button>
            ) : null}
            <Button disabled={blocked} onClick={flow.status}>
              Refresh status
            </Button>
            <span className="spacer" />
            {primary === 'open' ? null : openButton}
            {primary === 'check' ? null : checkButton}
            {primaryButton}
          </>
        }
      >
        <SsoStatus progress={progress} />
        {flow.hardwareRequired ? (
          <Inset className="form">{pinRow}</Inset>
        ) : null}
        {errorLine}
      </PanelSheet>
    );
  }

  const linking = progress?.purpose === 'link-existing';
  return (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      glyph={glyph}
      step="Step 1 of 2"
      footer={
        <>
          <Button disabled={blocked} onClick={flow.checkLinkage}>
            Check linkage
          </Button>
          <span className="spacer" />
          <Button disabled={busy} onClick={presentation.onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            title={flow.eligibility.title}
            disabled={flow.beginDisabled}
            onClick={flow.begin}
          >
            {linking
              ? 'Link existing account'
              : progress && !linkage
                ? 'Sign in again'
                : 'Sign in'}
          </Button>
        </>
      }
    >
      <p>
        Sign in through your organization’s identity provider to restore access
        on this device.
      </p>
      {flow.eligibility.title ? (
        <p role="status">{flow.eligibility.title}</p>
      ) : null}
      {progress ? <SsoStatus progress={progress} /> : null}
      {errorLine}
      <Inset className="form">
        <InsetRow
          label="Account"
          action={linkage ? <SsoStateChip progress={linkage} /> : undefined}
        >
          {account}
        </InsetRow>
        <InsetRow
          label="Security key"
          action={
            <input
              type="checkbox"
              aria-label="Security key"
              checked={flow.hardware || flow.hardwareRequired}
              disabled={blocked || flow.hardwareRequired}
              onChange={(event) => flow.chooseHardware(event.target.checked)}
            />
          }
        >
          <span className="dim">
            This account’s keys are on a security key.
          </span>
        </InsetRow>
        {flow.hardware || flow.hardwareRequired ? pinRow : null}
      </Inset>
    </PanelSheet>
  );
}
