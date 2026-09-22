/** Creates a named team or an ad-hoc share. */

import type { ReactNode } from 'react';
import {
  Button,
  Field,
  Icon,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../../components';
import {
  storeOperationAvailability,
  serverDisplayLabelForStore as displayServerName,
} from '../../model';
import type { AccountStore } from '../../model';
import { useSheetGuard, useTabSheetState } from '../../navigation-guard';
import { markProfileRostersStale } from '../../roster-staleness';
import {
  serverTeamName,
  SUGGESTED_GROUP,
  teamAliasOf,
  useSheetWrite,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';

type Kind = 'named' | 'adhoc';

export function CreateTeamSheet({
  snapshot,
  bridge,
  store,
  onClose,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps): ReactNode {
  const [name, setName] = useTabSheetState('group.name', '');
  const [kind, setKind] = useTabSheetState<Kind>('group.createKind', 'named');
  const creationAccounts = snapshot.stores.filter(
    (candidate): candidate is AccountStore =>
      candidate.kind === 'account' &&
      storeOperationAvailability(snapshot, candidate, 'teams').available,
  );
  // Default to the current account when it can create teams, then fall back to
  // the first eligible account.
  const [accountStoreId, setAccountStoreId] = useTabSheetState(
    'group.accountStoreId',
    () =>
      creationAccounts.find((candidate) => candidate.id === store.id)?.id ??
      creationAccounts[0]?.id ??
      '',
  );
  const { busy, write } = useSheetWrite(onApplied, onClose);
  const creationAccount = creationAccounts.find(
    (candidate) => candidate.id === accountStoreId,
  );
  const teamAlias = teamAliasOf(name);
  // Validate the name with the same normalization rules as the client.
  const reservedName = serverTeamName(name);
  const named = kind === 'named';
  const title = 'Create a team';
  useSheetGuard(
    busy
      ? null
      : name.trim()
        ? {
            verdict: 'prompt',
            title: 'Discard this team?',
            body: `${name.trim()} has not been created.`,
            confirm: 'Discard',
            onConfirm: () => {
              setName('');
              onClose();
            },
          }
        : null,
    !busy,
  );
  const ready =
    Boolean(teamAlias) &&
    Boolean(creationAccount) &&
    (!named || Boolean(reservedName));
  const apply = (): Promise<void> =>
    write(
      title,
      async () => {
        // Resolve the account again at submission time in case its
        // availability changed while the sheet was open.
        const account = creationAccounts.find(
          (candidate) => candidate.id === accountStoreId,
        );
        if (!account)
          throw new Error('No account store is available for team creation.');
        markProfileRostersStale(account.server);
        await bridge.createGroup({
          accountStoreId: account.id,
          teamAlias,
          name: named ? name : '',
          kind,
        });
        return {
          created: { accountStoreId: account.id, teamAlias },
          profile: account.server,
        };
      },
      (error) => onMutationError(error),
    );
  return (
    <SheetDialog
      onClose={onClose}
      dismissible={!busy}
      // Use a neutral icon because no team mark exists before creation.
      glyph={
        <span className="kico md neutral">
          <Icon name="users" />
        </span>
      }
      title={title}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={busy || !ready}
            onClick={() => void apply()}
          >
            {named ? 'Create team' : 'Create share'}
          </Button>
        </>
      }
    >
      <Inset>
        <RadioGroup label="Kind">
          <RadioCard
            selected={named}
            onSelect={() => setKind('named')}
            title="Named team"
            detail="Has a name on the server. People can be added and removed over time."
          />
          <RadioCard
            selected={!named}
            onSelect={() => setKind('adhoc')}
            title="Ad-hoc share"
            detail="No name on the server and a membership that does not change. For a one-off share."
          />
        </RadioGroup>
      </Inset>
      <SectionLabel>{named ? 'Name' : 'Local name'}</SectionLabel>
      <Inset>
        <Field
          label="Name"
          value={name}
          placeholder={SUGGESTED_GROUP}
          onChange={setName}
        />
      </Inset>
      <p className="fn">
        {named ? (
          reservedName ? (
            <>
              Named <code>{reservedName}</code> on the server and stored as{' '}
              <code>{teamAlias}</code>. Neither can be changed once created.
            </>
          ) : (
            'Use 3 to 25 letters, numbers, spaces, dots, dashes, or underscores.'
          )
        ) : (
          <>
            Only this device sees the name. Stored as{' '}
            <code>{teamAlias || '…'}</code>. Cannot be changed once created.
          </>
        )}
      </p>
      <SectionLabel>Server and account</SectionLabel>
      <Inset>
        {creationAccounts.length ? (
          <RadioGroup label="Server and account">
            {creationAccounts.map((account) => (
              <RadioCard
                key={account.id}
                selected={account.id === accountStoreId}
                onSelect={() => setAccountStoreId(account.id)}
                title={displayServerName(snapshot, account)}
                detail={`as ${snapshot.accounts.find((candidate) => candidate.store === account.id || (candidate.alias === account.account && candidate.server === account.server))?.username ?? account.account} · ${account.account} account`}
              />
            ))}
          </RadioGroup>
        ) : (
          <InsetRow label="Account">
            <span className="dim">No account can create a team.</span>
          </InsetRow>
        )}
      </Inset>
    </SheetDialog>
  );
}
