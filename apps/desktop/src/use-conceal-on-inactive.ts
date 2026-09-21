import { useEffect, useLayoutEffect, useRef } from 'react';

/**
 * Runs one concealment action when the window loses focus or the document is
 * hidden. A blur followed by visibilitychange is one inactive transition, so
 * it invokes the action only once until the window is visible and focused
 * again.
 */
export function useConcealOnInactive(
  onConceal: () => void,
  enabled = true,
): void {
  const latest = useRef(onConceal);
  const active = useRef(enabled);
  const inactive = useRef(false);
  const concealed = useRef(false);
  latest.current = onConceal;
  active.current = enabled;

  useEffect(() => {
    const conceal = (): void => {
      inactive.current = true;
      if (!active.current || concealed.current) return;
      concealed.current = true;
      latest.current();
    };
    const focus = (): void => {
      if (document.hidden) return;
      inactive.current = false;
      concealed.current = false;
    };
    const visibility = (): void => {
      if (document.hidden) conceal();
      else if (document.hasFocus()) focus();
    };
    window.addEventListener('blur', conceal);
    window.addEventListener('focus', focus);
    document.addEventListener('visibilitychange', visibility);
    return () => {
      window.removeEventListener('blur', conceal);
      window.removeEventListener('focus', focus);
      document.removeEventListener('visibilitychange', visibility);
    };
  }, []);

  // A secret can arrive after the blur that began an asynchronous read. Do
  // not let enabling its surface while still inactive reveal it.
  useLayoutEffect(() => {
    if (!enabled || !inactive.current || concealed.current) return;
    concealed.current = true;
    latest.current();
  }, [enabled]);
}
