import { useRef, type ReactNode } from 'react';
import { Icon } from '../../components';
import type { FirstRunPath } from '../../first-run-state';
import type { FoksIconName } from '../../icons';

/** Setup options for the initial screen: creating a new vault or joining an existing team. */
const JOINING_OPTIONS: readonly {
  path: FirstRunPath;
  icon: FoksIconName;
  title: string;
  detail: string;
}[] = [
  {
    path: 'own',
    icon: 'user',
    title: 'Set up my own account',
    detail: 'Set up your account and Personal vault.',
  },
  {
    path: 'invited',
    icon: 'users',
    title: 'Join an existing team',
    detail: 'Accept an invitation to join someone else’s team.',
  },
];

export function JoiningChoice({
  value,
  onChange,
}: {
  value: FirstRunPath | null;
  onChange: (path: FirstRunPath) => void;
}): ReactNode {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  // Radio group keyboard navigation.
  const move = (from: number, step: number): void => {
    const next =
      (from + step + JOINING_OPTIONS.length) % JOINING_OPTIONS.length;
    onChange(JOINING_OPTIONS[next].path);
    refs.current[next]?.focus();
  };
  return (
    <div
      className="setup-method-options"
      role="radiogroup"
      aria-label="Setup method"
    >
      {JOINING_OPTIONS.map((option, index) => {
        const on = value === option.path;
        return (
          <button
            key={option.path}
            ref={(node) => {
              refs.current[index] = node;
            }}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={on || (value === null && index === 0) ? 0 : -1}
            className={on ? 'setup-method-option on' : 'setup-method-option'}
            onClick={() => onChange(option.path)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
                event.preventDefault();
                move(index, 1);
              } else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
                event.preventDefault();
                move(index, -1);
              }
            }}
          >
            <span className="pip" aria-hidden="true" />
            <Icon name={option.icon} className="ic" />
            <span className="txt">
              <span className="ptitle">{option.title}</span>
              <span className="bd">{option.detail}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
