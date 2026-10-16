import type { ReactNode } from 'react';
import { Button, Icon, SectionLabel } from '../../components';
import { Foot, Pane } from './first-run-view';
import { useSetupContext } from './setup-context';

/** The private server this app runs on this device, offered as the default. */
export function LocalServerStep(): ReactNode {
  const { send, server } = useSetupContext();
  const {
    managedReport,
    managedStatus,
    managedStatusError,
    selectManagedProfile,
    setManagedStatusAttempt,
  } = server;
  return (
    <Pane
      title="Set up FOKS"
      header={false}
      foot={
        <Foot>
          <Button
            variant="primary"
            disabled={!managedReport}
            onClick={() => selectManagedProfile()}
          >
            Continue
          </Button>
        </Foot>
      }
    >
      <h1>Set up FOKS</h1>
      <p className="lead">
        A private FOKS server is already running on this device. Use it to
        create your Personal vault.
      </p>
      <div className="local-server-card">
        <div className="local-server-head">
          <span className="local-server-mark" aria-hidden="true">
            <Icon name="server" />
          </span>
          <span className="local-server-title">
            <b>Local server</b>
            <small>On this device</small>
          </span>
          <span className="local-ready">
            <i />
            {managedReport ? 'Ready' : 'Checking'}
          </span>
        </div>
        <dl className="local-server-facts">
          <dt>Address</dt>
          <dd>
            <code>{managedStatus?.configuredEndpoint ?? 'localhost:4430'}</code>
          </dd>
          <dt>Trust</dt>
          <dd>App-managed certificate</dd>
          <dt>Storage</dt>
          <dd>Local only</dd>
        </dl>
      </div>
      {managedStatusError ? (
        <div className="crit" role="alert">
          <b>Local server unavailable</b>
          {managedStatusError}
          <div className="btns">
            <Button
              onClick={() => setManagedStatusAttempt((value) => value + 1)}
            >
              Check again
            </Button>
          </div>
        </div>
      ) : null}
      <div className="local-alternates">
        <SectionLabel>Other options</SectionLabel>
        <div className="btns">
          <Button onClick={() => send({ type: 'choose', path: 'own' })}>
            Connect to another server…
          </Button>
          <Button
            disabled={!managedReport}
            onClick={() => selectManagedProfile(true)}
          >
            Recover an existing account…
          </Button>
        </div>
      </div>
    </Pane>
  );
}
