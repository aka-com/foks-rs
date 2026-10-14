import { useEffect, useLayoutEffect, useRef } from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Icon } from '../components';
import { TEXT_LIMIT_LABEL, useChatDraft } from './use-chat-composer';

/**
 * The message field and Send button of a channel the reader can write to.
 * Every key goes to `onKeyDown` first; then Enter sends, and Shift+Enter or
 * Alt+Enter inserts a line break.
 *
 * The composer reads the channel's draft itself, so a keystroke re-renders
 * the composer alone and not the thread or its message list.
 */
export function Composer({
  storeId,
  channel,
  placeholder,
  setDraft,
  canSend,
  onSend,
  onKeyDown,
  autofocus,
  fieldRef,
}: {
  /** The team, as the location addresses it. */
  storeId: string;
  /** The channel whose draft the field edits. */
  channel: string;
  placeholder: string;
  setDraft: (text: string) => void;
  canSend: boolean;
  onSend: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  autofocus?: (element: HTMLTextAreaElement) => void;
  /** The caller's handle on the message field, which Quote reply writes to. */
  fieldRef?: RefObject<HTMLTextAreaElement | null>;
}): ReactNode {
  const { draft, draftBytes, overLimit, nearLimit } = useChatDraft(
    storeId,
    channel,
  );
  const ownField = useRef<HTMLTextAreaElement>(null);
  const field = fieldRef ?? ownField;
  useComposerSize(draft, field);
  useEffect(() => {
    if (canSend && field.current) autofocus?.(field.current);
  }, [autofocus, canSend, field]);
  return (
    <form
      className="chat-composer"
      onSubmit={(e) => {
        e.preventDefault();
        onSend();
      }}
    >
      <textarea
        ref={field}
        aria-label="Message"
        value={draft}
        placeholder={placeholder}
        rows={1}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          onKeyDown?.(e);
          if (
            e.key !== 'Enter' ||
            e.nativeEvent.isComposing ||
            e.keyCode === 229
          )
            return;
          if (e.altKey) {
            e.preventDefault();
            const target = e.currentTarget;
            target.setRangeText(
              '\n',
              target.selectionStart,
              target.selectionEnd,
              'end',
            );
            setDraft(target.value);
          } else if (!e.shiftKey) {
            e.preventDefault();
            onSend();
          }
        }}
      />
      <button
        className="chat-send"
        type="submit"
        aria-label="Send"
        title="Send"
        disabled={!canSend || overLimit || !draft.trim()}
      >
        <Icon name="send" size={16} />
      </button>
      {nearLimit && (
        <small
          className={overLimit ? 'chat-meter over' : 'chat-meter'}
          aria-live="polite"
        >
          {Math.ceil(draftBytes / 1024)} KiB of {TEXT_LIMIT_LABEL}
        </small>
      )}
    </form>
  );
}

function resizeComposer(element: HTMLTextAreaElement): void {
  // Reset first so deleting text or clearing a sent draft shrinks the field.
  element.style.height = 'auto';
  const border = element.offsetHeight - element.clientHeight;
  element.style.height = `${element.scrollHeight + border}px`;
}

function useComposerSize(
  draft: string,
  ref: RefObject<HTMLTextAreaElement | null>,
): void {
  useLayoutEffect(() => {
    if (ref.current) resizeComposer(ref.current);
  }, [draft, ref]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    let width = element.clientWidth;
    const observer = new ResizeObserver(() => {
      // Reflow when the window or channel-info panel changes the available
      // width, without responding to our own height adjustments.
      if (element.clientWidth === width) return;
      width = element.clientWidth;
      resizeComposer(element);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
}
