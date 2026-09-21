import type { ReactNode } from 'react';
import { Button } from '../components';
import type { useAppBootstrap } from './app-bootstrap';
import { AgentStopCard, BlockedShell, type ShellBlock } from './blocking-shell';

export function BootstrapScreen({
  boot,
  block,
}: {
  boot: ReturnType<typeof useAppBootstrap>;
  block: ShellBlock;
}): ReactNode {
  const { activeBridge, lockState, lockError, unlocking } = boot;
  if (block.kind === 'locked' && lockState && activeBridge) {
    const mechanism =
      lockState.mechanism === 'biometry'
        ? 'Touch ID or your Mac password'
        : 'your operating-system password';
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <div className="tcard lockcard">
          <h2 id="app-lock-title">FOKS is locked</h2>
          <p>Authenticate with {mechanism}.</p>
          {lockError ? (
            <p className="fn bad" role="alert">
              {lockError}
            </p>
          ) : null}
          <div className="tacts">
            <Button
              variant="primary"
              disabled={unlocking}
              onClick={boot.unlock}
            >
              Unlock
            </Button>
          </div>
        </div>
      </BlockedShell>
    );
  }
  if (block.kind === 'boot-error') {
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        {/* A startup failure is an error takeover, so it takes the stop
            card's shape — a reported reason it can scroll, inline actions —
            rather than the lock card's single full-width control. */}
        <div className="tcard stopcard">
          <h2 id="app-boot-error-title">FOKS could not start</h2>
          {/* The card states the condition in its own words and the reported
              reason in the footnote, as the other takeover cards do. */}
          <p>
            The startup read did not complete. Retry to start FOKS again. Your
            vaults remain on this device and on their configured servers.
          </p>
          <p className="fn bad" role="alert">
            {block.message}
          </p>
          <div className="tacts">
            <Button variant="primary" onClick={boot.retry}>
              Retry
            </Button>
          </div>
        </div>
      </BlockedShell>
    );
  }
  if (block.kind === 'stop' && activeBridge) {
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <AgentStopCard
          lifecycle={block.lifecycle}
          bridge={activeBridge}
          onRetryRestoration={boot.retryRestoration}
        />
      </BlockedShell>
    );
  }
  return (
    <BlockedShell
      bridge={activeBridge}
      block={block.kind === 'starting' ? block : { kind: 'starting' }}
    />
  );
}
