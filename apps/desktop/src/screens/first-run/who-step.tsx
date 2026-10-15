import type { ReactNode } from 'react';
import { Button, Inset, RadioCard, RadioGroup } from '../../components';
import { Pane } from '../first-run-view';
import { GoProfileChooser } from '../go-profile-chooser';
import { JoiningChoice } from './setup-method-choice';
import { useSetupContext } from './setup-context';

/**
 * The first step: while the scan for FOKS CLI profiles runs, its progress;
 * when it found any, the choice between them and a new account; otherwise,
 * how the user is joining.
 */
export function WhoStep(): ReactNode {
  const { checkpoint, agentReady, bridge, cli } = useSetupContext();
  if (
    checkpoint.state === 'who' &&
    agentReady &&
    bridge.native &&
    cli.scanPending
  )
    return <CliScan />;
  if (cli.chooserOpen) return <CliProfileChooser />;
  return <JoiningMethod />;
}

function CliScan(): ReactNode {
  const { cli } = useSetupContext();
  return (
    <Pane title="Checking this device" header={false}>
      <h1>Looking for existing FOKS accounts</h1>
      <p className="lead">
        Checking for existing accounts from the official FOKS CLI. No changes
        will be made to your data.
      </p>
      <div className="setup-actions">
        <Button variant="primary" disabled busy>
          Checking…
        </Button>
        <Button onClick={cli.dismissChooser}>Skip for now</Button>
      </div>
    </Pane>
  );
}

function CliProfileChooser(): ReactNode {
  const { cli, server, identity, send } = useSetupContext();
  const { start, setStart, candidate } = cli;
  return (
    <Pane title="Set up FOKS" header={false} wide>
      <h1>Set up FOKS</h1>
      <p className="lead">
        A FOKS CLI account was found on this Mac. You can use it here or set up
        a new one.
      </p>
      <Inset>
        <RadioGroup label="How to start">
          <div className="choice">
            <RadioCard
              title="Use an existing account"
              detail="Continue with an account already active in the CLI."
              selected={start === 'existing'}
              onSelect={() => setStart('existing')}
            />
            {start === 'existing' ? (
              <div className="choice-body">
                <GoProfileChooser
                  nested
                  candidates={cli.candidates}
                  selected={candidate?.candidateId ?? null}
                  onSelect={cli.choose}
                />
              </div>
            ) : null}
          </div>
          <div className="choice">
            <RadioCard
              title="Create a new account"
              detail="Generate a new cryptographic identity and vault account."
              selected={start === 'new'}
              onSelect={() => setStart('new')}
            />
          </div>
        </RadioGroup>
      </Inset>
      <div className="setup-actions">
        <Button
          variant="primary"
          disabled={start === 'existing' && !candidate}
          onClick={() => {
            if (start === 'new') {
              cli.startNew();
              return;
            }
            const confirmed = cli.confirm();
            if (!confirmed) return;
            server.setAddress(confirmed.serverHint ?? '');
            identity.setUsername(
              confirmed.username?.toLowerCase().replace(/[^a-z0-9_-]+/g, '-') ??
                'personal',
            );
            send({ type: 'choose', path: 'own', returning: true });
          }}
        >
          Continue
        </Button>
      </div>
    </Pane>
  );
}

function JoiningMethod(): ReactNode {
  const { agentReady, busy, message, setMessage, send, cli, joining } =
    useSetupContext();
  return (
    <Pane title="How are you joining?" header={false} wide>
      <h1>How are you joining?</h1>
      <p className="lead">
        Choose an option to get started. You will set up your account and server
        in the next steps.
      </p>
      {cli.scanError ? (
        <div className="crit" role="alert">
          Could not check existing CLI profiles: {cli.scanError}
          <Button onClick={cli.retryScan}>Check again</Button>
        </div>
      ) : null}
      {message ? (
        <div className="crit" role="alert">
          <p>Setup could not be completed: {message}</p>
          <Button
            size="sm"
            onClick={() => {
              setMessage(null);
            }}
          >
            Try again
          </Button>
        </div>
      ) : null}
      <JoiningChoice value={joining.path} onChange={joining.setPath} />
      <div className="setup-actions">
        <Button
          variant="primary"
          disabled={!joining.path || !agentReady || busy !== null}
          onClick={() => {
            if (joining.path && agentReady)
              send({ type: 'choose', path: joining.path });
          }}
        >
          {!agentReady ? 'Preparing service…' : 'Continue'}
        </Button>
        {cli.chooserDismissed && cli.candidates.length > 0 ? (
          <Button onClick={cli.reopenChooser}>Back</Button>
        ) : null}
      </div>
    </Pane>
  );
}
