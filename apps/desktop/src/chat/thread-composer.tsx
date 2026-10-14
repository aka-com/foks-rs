import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Icon } from '../components';
import { CHAT_TEXT_BYTES } from '../chat-limits';
import { AccountMark } from '../screens/account-switcher';
import { parseRole, roleName } from '../model';
import {
  applePlatform,
  formatEdit,
  mentionEdit,
  mentionToken,
  textBudget,
  type ComposerEdit,
  type ComposerFormat,
  type MentionToken,
} from './composer-format';
import { ComposerHelp } from './composer-help';
import { matchMembers, type ChannelRoster } from './mentions';
import { useChatDraft } from './use-chat-composer';

/**
 * The message field and Send button of a channel the reader can write to,
 * with a hint line under them. Every key goes to `onKeyDown` first. Then,
 * while the `@name` completion is open, Up, Down, Enter, Tab and Esc work the
 * list; otherwise Enter sends, Shift+Enter or Alt+Enter inserts a line break,
 * Ctrl/⌘+B, I and E wrap the selection in `**`, `*` and `` ` `` (or remove
 * them), Ctrl/⌘+Shift+E puts the selected lines in a ``` fence, and Up in an
 * empty field calls `onRestore`.
 *
 * The composer reads the channel's draft itself, so a keystroke re-renders
 * the composer alone and not the thread or its message list.
 */
export function Composer({
  storeId,
  channel,
  channelTitle,
  placeholder,
  roster,
  setDraft,
  canSend,
  onSend,
  canRestore = false,
  onRestore,
  onKeyDown,
  autofocus,
  fieldRef,
}: {
  /** The team, as the location addresses it. */
  storeId: string;
  /** The channel whose draft the field edits. */
  channel: string;
  /** The channel as the completion's caption names it, `#general`. */
  channelTitle: string;
  placeholder: string;
  /** The members `@name` completes to: those who can read the channel. */
  roster: ChannelRoster;
  setDraft: (text: string) => void;
  canSend: boolean;
  onSend: () => void;
  /** The channel holds an unsent message that Up in an empty field edits. */
  canRestore?: boolean;
  onRestore?: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  autofocus?: (element: HTMLTextAreaElement) => void;
  /** The caller's handle on the message field, which Quote reply writes to. */
  fieldRef?: RefObject<HTMLTextAreaElement | null>;
}): ReactNode {
  const { draft, draftBytes, overLimit } = useChatDraft(storeId, channel);
  const ownField = useRef<HTMLTextAreaElement>(null);
  const field = fieldRef ?? ownField;
  const helpButton = useRef<HTMLButtonElement>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const hintId = useId();
  const listId = useId();
  useComposerSize(draft, field);
  useEffect(() => {
    if (canSend && field.current) autofocus?.(field.current);
  }, [autofocus, canSend, field]);

  // The `@name` at the caret, read again whenever the text or the caret
  // moves. Esc closes the list for that `@` until another one is typed.
  const [token, setToken] = useState<MentionToken | null>(null);
  const [dismissed, setDismissed] = useState<number | null>(null);
  const [choice, setChoice] = useState(0);
  const readCaret = (element: HTMLTextAreaElement) => {
    const next = mentionToken(
      element.value,
      element.selectionStart,
      element.selectionEnd,
    );
    if (next?.start !== token?.start) setChoice(0);
    if (!next) setDismissed(null);
    if (next?.start !== token?.start || next?.query !== token?.query)
      setToken(next);
  };
  const matches =
    token && token.start !== dismissed
      ? matchMembers(roster.members, token.query)
      : [];
  const open = matches.length > 0;
  const active = Math.min(choice, matches.length - 1);
  const optionId = (index: number) => `${listId}-${index}`;

  const apply = (element: HTMLTextAreaElement, edit: ComposerEdit) => {
    element.focus();
    element.setSelectionRange(edit.from, edit.to);
    // insertText keeps the edit on the field's undo stack, so Ctrl/⌘+Z
    // removes a wrap; where it is unavailable the text is replaced directly.
    let inserted: boolean;
    try {
      inserted = document.execCommand('insertText', false, edit.text);
    } catch {
      inserted = false;
    }
    if (
      !inserted ||
      element.value.slice(edit.from, edit.from + edit.text.length) !== edit.text
    )
      element.setRangeText(edit.text, edit.from, edit.to, 'end');
    element.setSelectionRange(edit.selectionStart, edit.selectionEnd);
    setDraft(element.value);
    readCaret(element);
  };
  const complete = (element: HTMLTextAreaElement, username: string) => {
    if (!token) return;
    apply(element, mentionEdit(token, username));
    setToken(null);
  };
  const formatKey = (
    event: KeyboardEvent<HTMLTextAreaElement>,
  ): ComposerFormat | null => {
    const command = applePlatform() ? event.metaKey : event.ctrlKey;
    if (!command || event.altKey) return null;
    const key = event.key.toLowerCase();
    if (key === 'e') return event.shiftKey ? 'codeBlock' : 'code';
    if (event.shiftKey) return null;
    return key === 'b' ? 'bold' : key === 'i' ? 'italic' : null;
  };

  const budget = textBudget(draftBytes, CHAT_TEXT_BYTES);
  return (
    <div className={helpOpen ? 'chat-compose help-open' : 'chat-compose'}>
      {open && (
        <div
          className="chat-mentions"
          // Keep focus in the field while an option is chosen with the pointer.
          onMouseDown={(event) => event.preventDefault()}
        >
          <p className="chat-mentions-caption">
            {roster.excluded
              ? `Members who can read ${channelTitle}`
              : 'Team members'}
          </p>
          <div role="listbox" id={listId} aria-label="Team members">
            {matches.map((member, index) => {
              const role = parseRole(member.role);
              return (
                <div
                  key={member.username}
                  id={optionId(index)}
                  role="option"
                  aria-selected={index === active}
                  className="chat-mention-option"
                  onClick={() => {
                    if (field.current) complete(field.current, member.username);
                  }}
                >
                  <AccountMark name={member.username} />
                  <span className="name">{member.username}</span>
                  {role && <span className="role">{roleName(role)}</span>}
                </div>
              );
            })}
          </div>
          {roster.excluded > 0 && (
            <p className="chat-mentions-note">
              <Icon name="key" size={12} />
              {roster.excluded === 1
                ? '1 team member cannot read this channel'
                : `${roster.excluded} team members cannot read this channel`}
            </p>
          )}
        </div>
      )}
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
          aria-describedby={hintId}
          aria-autocomplete="list"
          aria-controls={open ? listId : undefined}
          aria-activedescendant={open ? optionId(active) : undefined}
          value={draft}
          placeholder={placeholder}
          rows={1}
          onChange={(e) => {
            setDraft(e.target.value);
            readCaret(e.target);
          }}
          onClick={(e) => readCaret(e.currentTarget)}
          onKeyUp={(e) => {
            if (
              !['ArrowUp', 'ArrowDown', 'Enter', 'Tab', 'Escape'].includes(
                e.key,
              )
            )
              readCaret(e.currentTarget);
          }}
          onBlur={() => setToken(null)}
          onKeyDown={(e) => {
            onKeyDown?.(e);
            if (
              e.defaultPrevented ||
              e.nativeEvent.isComposing ||
              e.keyCode === 229
            )
              return;
            const target = e.currentTarget;
            const plain = !e.altKey && !e.ctrlKey && !e.metaKey;
            if (open && plain) {
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault();
                const step = e.key === 'ArrowDown' ? 1 : -1;
                setChoice((active + step + matches.length) % matches.length);
                return;
              }
              if ((e.key === 'Enter' || e.key === 'Tab') && !e.shiftKey) {
                e.preventDefault();
                complete(target, matches[active].username);
                return;
              }
              if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                setDismissed(token?.start ?? null);
                return;
              }
            }
            const format = formatKey(e);
            if (format) {
              e.preventDefault();
              apply(
                target,
                formatEdit(
                  format,
                  target.value,
                  target.selectionStart,
                  target.selectionEnd,
                ),
              );
              return;
            }
            if (
              e.key === 'ArrowUp' &&
              plain &&
              !e.shiftKey &&
              target.value === '' &&
              canRestore
            ) {
              e.preventDefault();
              onRestore?.();
              return;
            }
            if (e.key !== 'Enter') return;
            if (e.altKey) {
              e.preventDefault();
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
      </form>
      <div className="chat-compose-hints">
        <p className="chat-compose-hint" id={hintId}>
          {!draft && canRestore ? (
            <>
              <kbd>↑</kbd> to edit your unsent message
              <span className="dot"> · </span>
              Enter to send
            </>
          ) : (
            <>
              Enter to send<span className="dot"> · </span>
              Shift+Enter new line
              <span className="chat-compose-formats">
                <span className="dot"> · </span>
                <code>**bold**</code> <code>*italic*</code> <code>`code`</code>
              </span>
            </>
          )}
        </p>
        {budget && (
          <small
            className={budget.over ? 'chat-meter over' : 'chat-meter'}
            title={budget.detail}
          >
            {budget.label}
          </small>
        )}
        <button
          type="button"
          ref={helpButton}
          className="chat-compose-help"
          aria-label="Formatting and keys"
          title="Formatting and keys"
          aria-expanded={helpOpen}
          onClick={() => setHelpOpen((shown) => !shown)}
        >
          ?
        </button>
      </div>
      {/* Read when the completion list opens or changes length, and when
          the message passes 90% of the size limit or the limit itself; not
          on every keystroke. */}
      <span className="offscreen" role="status">
        {open
          ? `${matches.length} ${matches.length === 1 ? 'suggestion' : 'suggestions'}. Up and Down to choose, Enter to insert.`
          : budget?.over
            ? 'This message is over the size limit. Shorten it to send.'
            : budget
              ? 'This message is near the size limit.'
              : ''}
      </span>
      {helpOpen && (
        <ComposerHelp
          anchor={helpButton}
          onClose={(restore) => {
            setHelpOpen(false);
            if (restore) helpButton.current?.focus();
          }}
        />
      )}
    </div>
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
