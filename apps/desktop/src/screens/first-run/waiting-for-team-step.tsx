import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  CopyBox,
  Icon,
  Inset,
  InsetRow,
} from '../../components';
import type { DiscoveredGroup } from '../../bridge';
import type {
  FirstRunCheckpoint,
  FirstRunStateName,
} from '../../first-run-state';
import type { TeamDiscoveryOutcome } from './use-team-discovery';
import { Foot, Pane } from '../first-run-view';

const PERSONAL_FIXED =
  'Your Personal vault is private to your account. To share items with others, use a team.';

export function WaitingForTeamStep({
  checkpoint,
  busy,
  message,
  admin,
  adminShort,
  group,
  discoveredGroups,
  discoveryOutcome,
  discover,
  selectDiscoveredGroup,
  go,
  onChooseAnotherTeam,
  onCopy,
}: {
  checkpoint: FirstRunCheckpoint;
  busy: boolean;
  message: string | null;
  admin: string;
  adminShort: string;
  group: string;
  discoveredGroups: DiscoveredGroup[];
  discoveryOutcome: TeamDiscoveryOutcome | null;
  discover: () => Promise<void>;
  selectDiscoveredGroup: (group: DiscoveredGroup) => void;
  go: (state: FirstRunStateName) => void;
  onChooseAnotherTeam: () => void;
  onCopy: (value: string) => void;
}): ReactNode {
  const profile = checkpoint.profile;
  if (checkpoint.selectedGroup)
    return (
      <Pane
        title="Team vault unavailable"
        foot={
          <Foot>
            <Button onClick={() => go('checklist-invited')}>
              Finish later
            </Button>
          </Foot>
        }
      >
        <h1>{checkpoint.selectedGroup.name}</h1>
        <p className="lead">
          {/* Reached either from a vault that could not be opened or from a
              team the catalog no longer binds, which the checkpoint does not
              tell apart; the retry below finds out which. */}
          {message ||
            'This team’s vault could not be opened. Retry to check your membership again, or choose another team.'}
        </p>
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={busy}
            busy={busy}
            onClick={() => void discover()}
          >
            {busy ? 'Retrying…' : 'Retry loading team'}
          </Button>
          <Button
            onClick={() => {
              onChooseAnotherTeam();
            }}
          >
            Choose another team
          </Button>
        </div>
      </Pane>
    );
  return (
    <Pane
      title={`Waiting for ${admin} to add ${checkpoint.account?.username}`}
      header={false}
      wide
      foot={
        <Foot>
          <Button onClick={() => go('checklist-invited')}>Finish later</Button>
        </Foot>
      }
    >
      <h1>
        Waiting for {admin} to add {checkpoint.account?.username}
      </h1>
      <p className="lead">
        Your account ({checkpoint.account?.username}) on{' '}
        {profile?.canonicalName} is ready. {adminShort} must add you to{' '}
        <b>{group}</b>. Once they have added you, select Check now to finish
        joining.
      </p>
      {discoveredGroups.length > 1 ? (
        <div className="pcard">
          <h3>Choose an existing team</h3>
          <p>
            You’re already a member of these teams. Open one to get started.
          </p>
          <div className="btns">
            {discoveredGroups.map((found) => (
              <Button
                key={`${found.kind}:${found.teamIdHex}:${found.alias}`}
                onClick={() => selectDiscoveredGroup(found)}
              >
                Open “{found.name ?? found.alias}”
              </Button>
            ))}
          </div>
        </div>
      ) : null}
      <div className="setup-grid">
        <div className="col">
          <div className="pcard">
            <h3>Message for {adminShort}</h3>
            <CopyBox
              text={`Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`}
              onCopy={(value) => onCopy(value)}
            >
              “Add {checkpoint.account?.username} on {profile?.canonicalName} to{' '}
              {group}”
            </CopyBox>
            <p>
              Send this message to {adminShort} so they have your exact username
              and server.
            </p>
          </div>
          <Inset className="checklist">
            <InsetRow label={<Icon name="users" />}>
              <b>{group} will appear under Teams</b>
              <span className="hint">
                Teams appear once membership is confirmed by the server.
              </span>
            </InsetRow>
            <InsetRow label={<Icon name="eye" />}>
              <b>Access depends on your team role</b>
              <span className="hint">
                Roles include <b>Member</b>, <b>Admin</b>, and <b>Owner</b>. You
                can only access items permitted by your assigned role and
                visibility level.
              </span>
            </InsetRow>
            <InsetRow label={<Icon name="door" />}>
              <b>You can close FOKS anytime</b>
              <span className="hint">
                Your account and server settings are saved on this device. When
                you reopen FOKS, you can continue setup.
              </span>
            </InsetRow>
          </Inset>
        </div>
        <div className="col">
          <div className="pcard">
            <h3>Check now</h3>
            <div className="checkrow">
              <Button
                variant="primary"
                disabled={busy}
                onClick={() => void discover()}
              >
                Check now
              </Button>
              <span className="status">
                {/* Only a check that completed and found nothing says the
                      team was not found; a failed check says it failed, and
                      the sentence below carries what went wrong. */}
                <Chip>
                  {!message
                    ? 'Not checked yet'
                    : discoveryOutcome === 'failed'
                      ? 'Check failed'
                      : 'Checked: now'}
                </Chip>
                {message && discoveryOutcome === 'not-found' ? (
                  <>Team not found yet. Only Personal is available.</>
                ) : null}
              </span>
            </div>
            <p>
              Select <b>Check now</b> to look for pending team invitations. FOKS
              will also check automatically each time you open the app.
            </p>
            <Band label="Team updates">
              <b>Check now</b> checks the server for team memberships linked to
              your account. FOKS also checks when it opens.
            </Band>
            {message ? <div className="res">{message}</div> : null}
            <p className="note">
              You can use <b>Personal</b> while you wait. If {adminShort} has
              already added you, confirm that they entered{' '}
              <code>{checkpoint.account?.username}</code>.
            </p>
            <details className="dd">
              <summary>
                <Icon name="chevronDown" />
                Details
              </summary>
              <p>
                Checks your account ({checkpoint.account?.username}) on{' '}
                {profile?.canonicalName} and updates your team list.
              </p>
            </details>
          </div>
          <div className="pcard">
            <h3>Use your Personal vault</h3>
            <p>
              {PERSONAL_FIXED} You can store private items in <b>Personal</b>{' '}
              right away; items in Personal are never shared with {group}.
            </p>
            <Button onClick={() => go('checklist-invited')}>
              Continue to my vault
            </Button>
          </div>
        </div>
      </div>
    </Pane>
  );
}
