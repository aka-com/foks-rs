/** Settings › Account's secondary workflows, below the server list. */

import { useWorkflowAccess } from '../../workflow-context';
import type { ReactNode } from 'react';
import { Button } from '../../components';
import type { AccountStore, AgentSnapshot } from '../../model';
import type { AccountSheet } from './account-panel';

/** Secondary account workflows, placed after the device-wide server list. */
export function AccountActions({
  snapshot,
  store,
  onSheet,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  onSheet: (sheet: AccountSheet) => void;
}): ReactNode {
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  return (
    <div className="fn account-more">
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('bot-list', target)}
        onClick={() => onSheet('bot')}
      >
        Bot accounts
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('web-admin-configure', target)}
        onClick={() => onSheet('admin')}
      >
        Web admin panel
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('sso-login', target)}
        onClick={() => onSheet('sso')}
      >
        Sign in via SSO
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        onClick={() => onSheet('go-profile')}
      >
        Import from FOKS CLI
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('account-recover', { profile: store.server })}
        onClick={() => onSheet('recover')}
      >
        Connect an existing account with a recovery phrase
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('yubi-create', { profile: store.server })}
        onClick={() => onSheet('enroll')}
      >
        Create an account on a hardware key…
      </Button>
    </div>
  );
}
