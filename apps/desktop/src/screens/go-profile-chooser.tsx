import { useId } from 'react';
import type { ReactNode } from 'react';
import type { GoProfileCandidate } from '../bridge';
import { Chip } from '../components';

interface Props {
  candidates: GoProfileCandidate[];
  selected: string | null;
  onSelect: (candidate: GoProfileCandidate) => void;
}

const shortId = (value: string): string =>
  `${value.slice(0, 8)}…${value.slice(-8)}`;

export function GoProfileChooser({
  candidates,
  selected,
  onSelect,
}: Props): ReactNode {
  const group = useId();
  return (
    <div
      className="go-account-options"
      role="radiogroup"
      aria-label="FOKS accounts"
    >
      {candidates.map((candidate) => {
        const unavailable = !candidate.pairable && !candidate.copyable;
        const status = candidate.provisional
          ? 'Incomplete'
          : candidate.hidden
            ? 'Hidden'
            : candidate.copyable
              ? 'Pair or import'
              : candidate.pairable
                ? 'Pair'
                : 'Unavailable';
        return (
          <label className="go-account-option" key={candidate.candidateId}>
            <input
              type="radio"
              name={group}
              aria-label={`Select account ${candidate.username ?? shortId(candidate.userId)} on device ${shortId(candidate.deviceId)}`}
              checked={selected === candidate.candidateId}
              disabled={unavailable}
              onChange={() => onSelect(candidate)}
            />
            <span className="go-account-name">
              {candidate.username ??
                `Unnamed account (${shortId(candidate.userId)})`}
            </span>
            <span className="go-account-detail">
              <span>
                {candidate.serverHint ??
                  `Server (${shortId(candidate.hostId)})`}{' '}
                <Chip>{status}</Chip>
              </span>
              <small>
                {candidate.role.toLowerCase() === 'owner'
                  ? 'Account owner'
                  : 'Member'}{' '}
                ·{' '}
                {candidate.storageKind.includes('keychain')
                  ? 'Keychain storage'
                  : 'Local storage'}
              </small>
            </span>
          </label>
        );
      })}
    </div>
  );
}
