import type { ReactNode } from 'react';
import type { GoProfileCandidate } from '../bridge';
import { Button, Chip, Inset, InsetRow } from '../components';

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
  return (
    <Inset className="settings-inset middle">
      {candidates.map((candidate) => {
        const unavailable = !candidate.pairable && !candidate.copyable;
        const status = candidate.provisional
          ? 'Incomplete'
          : candidate.hidden
            ? 'Hidden'
            : candidate.copyable
              ? 'Pair or copy'
              : candidate.pairable
                ? 'Pair'
                : 'Unavailable';
        return (
          <InsetRow
            key={candidate.candidateId}
            label={candidate.username ?? `Account ${shortId(candidate.userId)}`}
            action={
              <Button
                size="sm"
                variant={
                  selected === candidate.candidateId ? 'primary' : undefined
                }
                disabled={unavailable}
                onClick={() => onSelect(candidate)}
              >
                {selected === candidate.candidateId ? 'Selected' : 'Select'}
              </Button>
            }
          >
            <span>
              {candidate.serverHint ?? `Server ${shortId(candidate.hostId)}`}{' '}
              <Chip>{status}</Chip>
            </span>
            <small>
              {candidate.role} · {candidate.storageKind} · device{' '}
              {shortId(candidate.deviceId)}
            </small>
          </InsetRow>
        );
      })}
    </Inset>
  );
}
