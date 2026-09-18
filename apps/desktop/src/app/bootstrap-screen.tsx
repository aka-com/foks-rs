import type { ReactNode } from 'react';
import { Button, Icon } from '../components';
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
        <div className="card lockcard">
          <span className="glyph" aria-hidden="true">
            <Icon name="shield" />
          </span>
          <h2 id="app-lock-title">FOKS is locked</h2>
          <p>Authenticate with {mechanism} to unlock FOKS.</p>
          {lockError ? (
            <p className="action-error" role="alert">
              {lockError}
            </p>
          ) : null}
          <Button variant="primary" disabled={unlocking} onClick={boot.unlock}>
            Unlock
          </Button>
        </div>
      </BlockedShell>
    );
  }
  if (block.kind === 'boot-error') {
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <div className="card lockcard">
          <span className="glyph warn" aria-hidden="true">
            <Icon name="alert" />
          </span>
          <h2 id="app-boot-error-title">Couldn’t load FOKS</h2>
          <p className="action-error" role="alert">
            {block.message}
          </p>
          <Button variant="primary" onClick={boot.retry}>
            Retry
          </Button>
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
  return <BlockedShell bridge={activeBridge} block={{ kind: 'starting' }} />;
}
