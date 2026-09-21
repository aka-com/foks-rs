import { useEffect, useState } from 'react';

export type OnboardingTip = 'add-password';
const EVENT = 'foks-onboarding-tip-change';
const keyFor = (tip: OnboardingTip) => `onboarding.dismissed.${tip}`;

function read(tip: OnboardingTip): boolean {
  try {
    return window.localStorage.getItem(keyFor(tip)) === 'true';
  } catch {
    return false;
  }
}

/** Local display preferences, independent of account setup completion. */
export function useDismissedOnboardingTip(tip: OnboardingTip) {
  const [dismissed, setDismissed] = useState(() => read(tip));
  useEffect(() => {
    const changed = (event: Event) => {
      const detail = (
        event as CustomEvent<{ tip: OnboardingTip; dismissed: boolean }>
      ).detail;
      if (detail.tip === tip) setDismissed(detail.dismissed);
    };
    const stored = (event: StorageEvent) => {
      if (event.key === null || event.key === keyFor(tip))
        setDismissed(read(tip));
    };
    window.addEventListener(EVENT, changed);
    window.addEventListener('storage', stored);
    return () => {
      window.removeEventListener(EVENT, changed);
      window.removeEventListener('storage', stored);
    };
  }, [tip]);
  const update = (value: boolean) => {
    try {
      if (value) window.localStorage.setItem(keyFor(tip), 'true');
      else window.localStorage.removeItem(keyFor(tip));
    } catch {
      /* Keep the preference for this mounted session. */
    }
    setDismissed(value);
    window.dispatchEvent(
      new CustomEvent(EVENT, { detail: { tip, dismissed: value } }),
    );
  };
  return [dismissed, update] as const;
}
