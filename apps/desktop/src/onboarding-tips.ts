import { useEffect, useState } from 'react';

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
  type DeviceStorageKey,
} from './platform/device-storage';

export type OnboardingTip = 'add-password' | 'setup-checklist';
const EVENT = 'foks-onboarding-tip-change';
const TIP_KEYS: Record<OnboardingTip, DeviceStorageKey> = {
  'add-password': STORAGE_KEYS.dismissedAddPasswordTip,
  'setup-checklist': STORAGE_KEYS.dismissedSetupChecklistTip,
};
const keyFor = (tip: OnboardingTip) => TIP_KEYS[tip];

function read(tip: OnboardingTip): boolean {
  return readStorage(keyFor(tip)) === 'true';
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
      if (event.key === null || event.key === keyFor(tip).name)
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
    // A refused write keeps the preference for this mounted session.
    writeStorage(keyFor(tip), value ? 'true' : null);
    setDismissed(value);
    window.dispatchEvent(
      new CustomEvent(EVENT, { detail: { tip, dismissed: value } }),
    );
  };
  return [dismissed, update] as const;
}
