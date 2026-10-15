/**
 * The Copy diagnostics action of the Refresh status popover and the window
 * fault screen.
 */

import { useEffect, useRef, useState } from 'react';

/**
 * Copies diagnostics text to the clipboard. The write starts during the click
 * so WebKit retains the user gesture. Where the clipboard accepts a
 * promise-backed item, `produce` supplies the text afterwards and can collect
 * fresh events; older clipboard implementations take `fallback`'s text,
 * computed during the click. Only a write the clipboard accepted sets
 * `copied`, which clears after one second.
 */
export function useCopyDiagnostics(
  produce: () => Promise<string>,
  fallback: () => string,
): { copied: boolean; copy: () => void } {
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  useEffect(
    () => () => {
      if (copiedTimer.current !== undefined) clearTimeout(copiedTimer.current);
    },
    [],
  );
  const copy = (): void => {
    const clipboard =
      typeof navigator === 'undefined' ? undefined : navigator.clipboard;
    if (!clipboard) return;
    let writing: Promise<void>;
    try {
      if (typeof ClipboardItem !== 'undefined' && clipboard.write) {
        const content = produce().then(
          (text) => new Blob([text], { type: 'text/plain' }),
        );
        writing = clipboard.write([
          new ClipboardItem({ 'text/plain': content }),
        ]);
      } else {
        writing = clipboard.writeText(fallback());
      }
    } catch {
      return;
    }
    void writing.then(
      () => {
        if (copiedTimer.current !== undefined)
          clearTimeout(copiedTimer.current);
        setCopied(true);
        copiedTimer.current = setTimeout(() => setCopied(false), 1_000);
      },
      () => undefined,
    );
  };
  return { copied, copy };
}
