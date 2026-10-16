import type { ReactNode } from 'react';
import { Button, Chip, KindIcon, Notice } from '../../components';
import type { FilterKind } from '../../components';
import { displayPath, kindOf, nameOf } from '../../model';
import { Pane } from './first-run-view';
import { readableBy } from '../../model/scope';
import { useSetupContext } from './setup-context';

/** The team setup joined, with the first of its items once its vault reads. */
export function TeamAddedStep(): ReactNode {
  const { checkpoint, snapshot, onNavigate, message, stores, personal } =
    useSetupContext();
  const { group, addedStore } = stores;
  return (
    <Pane
      title={group}
      subtitle={`Team on ${checkpoint.profile?.canonicalName}`}
      wide
    >
      <Notice
        // The pane's title already names the team, so the notice states the
        // condition alone, in the same words the store access takeover uses.
        severity={addedStore ? 'info' : 'warn'}
        title={addedStore ? `Joined ${group}` : 'Vault unavailable'}
        actions={
          <>
            <Button onClick={() => onNavigate({ kind: 'all' })}>Dismiss</Button>
            {!addedStore ? (
              <Button
                disabled={personal.refreshing}
                onClick={personal.retryTeam}
              >
                Retry loading team
              </Button>
            ) : null}
            <Button
              variant="primary"
              disabled={!addedStore}
              onClick={() => {
                if (addedStore) onNavigate({ kind: 'store', ref: addedStore });
              }}
            >
              Open team
            </Button>
          </>
        }
      >
        <p>
          You have been added to this team as{' '}
          <code>{checkpoint.account?.username}</code>. You can now access the
          items listed below when the vault is available.
        </p>
        {message ? <p role="status">{message}</p> : null}
      </Notice>
      <div className="hdr">
        <span />
        <span>Name</span>
        <span>Access</span>
        <span>Version</span>
      </div>
      {snapshot.items
        .filter((item) => item.store === addedStore)
        .slice(0, 4)
        .map((item) => (
          <div className="row" key={`${item.store}|${item.path}`}>
            <KindIcon kind={kindOf(item) as FilterKind} />
            <span className="name">
              {nameOf(item.path)}
              <small>{displayPath(item.path)}</small>
            </span>
            <span>
              <Chip>{readableBy(snapshot, item).label}</Chip>
            </span>
            <span className="n">v{item.version}</span>
          </div>
        ))}
    </Pane>
  );
}
