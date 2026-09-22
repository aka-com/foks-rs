/** Adds an existing account on the team's server as a member. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { RoleDto } from '../../bridge';
import { normalizeCommandError } from '../../bridge';
import {
  Button,
  Chip,
  Field,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../../components';
import {
  groupDetailFailure,
  partiesOf,
  partyName,
  roleRank,
  serverDisplayLabelForStore as displayServerName,
} from '../../model';
import { useSheetGuard, useTabSheetState } from '../../navigation-guard';
import { markProfileRostersStale } from '../../roster-staleness';
import { GroupMark } from '../group-mark';
import {
  DetailUnavailable,
  useSheetWrite,
  VisibilityStepper,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';

export function AddPersonSheet({
  snapshot,
  bridge,
  store,
  onClose,
  onInvite,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps & {
  /** Opens the invitation flow for a user without an account. */
  onInvite?: () => void;
}): ReactNode {
  const [username, setUsername] = useTabSheetState('group.username', '');
  const [visibility, setVisibility] = useTabSheetState('group.visibility', 0);
  const [role, setRole] = useTabSheetState<RoleDto>('group.role', {
    role: 'Member',
    visibility: 0,
  });
  const [refused, setRefused] = useState('');
  const { busy, write } = useSheetWrite(onApplied, onClose);
  const callerParty = partiesOf(snapshot, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const callerRank = callerParty ? roleRank(callerParty.destination_role) : 0;
  const serverName = displayServerName(snapshot, store);
  const requiredFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const title = `Add FOKS user to ${store.name}`;
  // Reject an existing roster member locally. The server validates all other
  // username errors.
  const existing = partiesOf(snapshot, store.id).find(
    (party) =>
      party.party_kind === 'user' &&
      (party.username ?? '').toLowerCase() === username.trim().toLowerCase(),
  );
  const addRefusal = username.trim()
    ? existing
      ? `${partyName(existing)} is already a member of ${store.name}. Change their role from the Members list instead.`
      : refused
    : '';
  // Confirm dismissal while the username contains an unsubmitted value.
  useSheetGuard(
    busy
      ? null
      : username.trim()
        ? {
            verdict: 'prompt',
            title: 'Discard this member?',
            body: `${username.trim()} has not been added to ${store.name}.`,
            confirm: 'Discard',
            onConfirm: () => {
              setUsername('');
              onClose();
            },
          }
        : null,
    !busy,
  );
  const apply = (): Promise<void> => {
    if (requiredFailure || existing) return Promise.resolve();
    setRefused('');
    return write(
      title,
      async () => {
        // Invalidate the roster before writing because an ambiguous error may
        // occur after the server applies the change.
        markProfileRostersStale(store.server);
        await bridge.addGroupMember({
          storeId: store.id,
          username: username.trim(),
          destination: role.role === 'Member' ? { ...role, visibility } : role,
        });
        return { profile: store.server };
      },
      async (error) => {
        // Display the server error in the sheet and suppress a duplicate
        // shell notification while reconciliation runs.
        setRefused(normalizeCommandError(error).message);
        await onMutationError(error, { report: false });
      },
    );
  };
  return (
    <SheetDialog
      onClose={onClose}
      dismissible={!busy}
      glyph={<GroupMark store={store} />}
      title={title}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={
              busy ||
              Boolean(requiredFailure) ||
              !username.trim() ||
              Boolean(existing)
            }
            onClick={() => void apply()}
          >
            Add {username.trim() || 'someone'}
          </Button>
        </>
      }
    >
      {requiredFailure ? <DetailUnavailable failure={requiredFailure} /> : null}
      <p>
        Add someone who already has an account on {serverName}. They get access
        as soon as this finishes.
      </p>
      <Inset>
        <Field
          label="Username"
          value={username}
          // Clear the server error when the submitted username changes.
          onChange={(next) => {
            setUsername(next);
            setRefused('');
          }}
        />
        {/* The target team determines the server. */}
        <InsetRow label="Server" action={<Chip>This team’s server</Chip>}>
          <span className="dim">{serverName}</span>
        </InsetRow>
      </Inset>
      {addRefusal ? (
        <p role="alert" className="action-error">
          {addRefusal}
        </p>
      ) : null}
      <SectionLabel>Role in {store.name}</SectionLabel>
      <Inset>
        <RadioGroup label={`Role in ${store.name}`}>
          {(['Owner', 'Admin', 'Member'] as const).map((next) => {
            // Keep Owner visible but disabled when the current user is not an
            // Owner.
            const refusal =
              next === 'Owner' && callerRank < 3
                ? 'Only an Owner can add another Owner.'
                : '';
            return (
              <RadioCard
                key={next}
                selected={role.role === next}
                off={Boolean(refusal)}
                onSelect={() =>
                  setRole(
                    next === 'Member'
                      ? { role: next, visibility }
                      : { role: next },
                  )
                }
                title={next}
                detail={
                  refusal ||
                  (next === 'Member'
                    ? 'Can only read items.'
                    : next === 'Admin'
                      ? 'Manage vault items and team members, excluding other admins and owners.'
                      : 'Manage vault items, members, permissions, or delete the team.')
                }
              />
            );
          })}
        </RadioGroup>
      </Inset>
      {role.role === 'Member' ? (
        <Inset className="visibility-inset">
          <InsetRow
            label="Visibility"
            action={
              <VisibilityStepper value={visibility} onChange={setVisibility} />
            }
          />
        </Inset>
      ) : null}
      {onInvite ? (
        <p className="fn">
          No account yet?{' '}
          <button type="button" className="lnk" onClick={onInvite}>
            Invite them to {serverName} instead…
          </button>
        </p>
      ) : null}
    </SheetDialog>
  );
}
