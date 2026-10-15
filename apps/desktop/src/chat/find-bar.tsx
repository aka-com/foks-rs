import { useId } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Button, Icon } from '../components';
import { plural } from '../model';
import { AccountMark } from '../components/account-mark';
import { chatErrorText } from './error-copy';
import {
  FIND_SCAN_PAGES,
  findDay,
  findSnippet,
  type FindReference,
} from './find';
import { messageTime, senderName } from './presentation';
import type { useChatFind } from './use-chat-find';

type Find = ReturnType<typeof useChatFind>;

/**
 * The find bar under the thread header. It states what it searched: the
 * count reads "No matches in loaded messages" rather than "No matches",
 * because older history has not been read, and "Search older messages" names
 * the day the held window reaches back to. Matches a scan of older pages
 * found are listed below it, and opening one opens the channel around it.
 */
export function FindBar({
  find,
  title,
  oldest,
  olderAvailable,
  actor,
  names,
  onOpen,
}: {
  find: Find;
  /** The channel's title, "#name". */
  title: string;
  /** The insertion time of the oldest row the window holds, if any. */
  oldest: string | null;
  /** The window has an older cursor: rows exist below it. */
  olderAvailable: boolean;
  actor: string | null;
  names: ReadonlyMap<string, string>;
  onOpen: (reference: FindReference) => void;
}): ReactNode {
  const countId = useId();
  const {
    query,
    total,
    current,
    scan,
    input,
    close,
    setQuery,
    step,
    searchOlder,
    stopOlder,
  } = find;
  const trimmed = query.trim();
  const label = `Find in ${title}`;
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      step(event.shiftKey ? 'newer' : 'older');
    }
  };
  const reached = scan.reached ? findDay(scan.reached) : null;
  const matches = plural(scan.hits.length, 'match', 'matches');
  let more: ReactNode = null;
  if (trimmed) {
    if (scan.state === 'scanning')
      more = (
        <>
          <span className="spin" aria-hidden="true" />
          <span role="status">
            Searching older messages · page{' '}
            {Math.min(scan.pages + 1, FIND_SCAN_PAGES)} of {FIND_SCAN_PAGES}
          </span>
          <Button size="sm" onClick={stopOlder}>
            Cancel
          </Button>
        </>
      );
    else if (scan.state === 'failed')
      more = (
        <>
          <span role="alert">
            Older messages could not be searched.{' '}
            {scan.failure ? chatErrorText(scan.failure) : ''}
          </span>
          <button
            type="button"
            className="lnk"
            onClick={() => void searchOlder()}
          >
            Try again
          </button>
        </>
      );
    else if (scan.state === 'done' || scan.state === 'stopped')
      more = (
        <>
          <span role="status">
            {scan.state === 'stopped'
              ? `Stopped after ${plural(scan.pages, 'page')}${reached ? `, back to ${reached}` : ''}`
              : scan.cursor === null
                ? `Searched to the beginning of ${title}`
                : `Searched ${plural(scan.scanned, 'older message')}${reached ? ` back to ${reached}` : ''}`}{' '}
            · {matches}
          </span>
          {scan.cursor !== null && (
            <button
              type="button"
              className="lnk"
              onClick={() => void searchOlder()}
            >
              Search further
            </button>
          )}
        </>
      );
    else if (olderAvailable)
      more = (
        <button
          type="button"
          className="lnk"
          onClick={() => void searchOlder()}
        >
          Search older messages
          {oldest ? ` (loaded back to ${findDay(oldest)})` : ''}
        </button>
      );
  }
  return (
    <div className="chat-find" role="search" aria-label={label}>
      <div className="chat-find-row">
        <label className="chat-find-field">
          <Icon name="search" />
          <input
            ref={input}
            type="text"
            value={query}
            placeholder={label}
            aria-label={label}
            aria-describedby={countId}
            aria-keyshortcuts="Enter Shift+Enter Escape"
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
          />
        </label>
        <span
          id={countId}
          className={
            trimmed && !total ? 'chat-find-count none' : 'chat-find-count'
          }
          role="status"
          aria-atomic="true"
        >
          {!trimmed
            ? ''
            : total
              ? `${current} of ${total}`
              : 'No matches in loaded messages'}
        </span>
        <Button
          variant="quiet"
          icon="chevronUp"
          aria-label="Older match"
          title="Older match (Enter)"
          disabled={total < 2}
          onClick={() => step('older')}
        />
        <Button
          variant="quiet"
          icon="chevronDown"
          aria-label="Newer match"
          title="Newer match (Shift+Enter)"
          disabled={total < 2}
          onClick={() => step('newer')}
        />
        <Button
          variant="quiet"
          icon="close"
          className="chat-find-close"
          aria-label="Close find"
          title="Close (Esc)"
          onClick={close}
        />
      </div>
      {more && <div className="chat-find-more">{more}</div>}
      {trimmed && scan.hits.length > 0 && (
        <div
          className="chat-find-refs"
          role="group"
          aria-label="Matches in older messages"
        >
          <div className="chat-find-refs-head" aria-hidden="true">
            In older messages<small>Opens around the message</small>
          </div>
          {scan.hits.map((hit) => {
            const author =
              actor !== null && hit.sender === actor
                ? 'You'
                : senderName(hit.sender, names);
            const snippet = findSnippet(hit.text, trimmed);
            return (
              <button
                type="button"
                className="chat-find-ref"
                key={hit.id}
                onClick={() => onOpen(hit)}
              >
                <AccountMark
                  name={senderName(hit.sender, names)}
                  className="chat-avatar"
                />
                <span className="t">
                  <span className="when">
                    <b>{author}</b> · {messageTime(hit.insert_time)}
                  </span>
                  {snippet && (
                    <span className="snip">
                      {snippet.before}
                      <strong>{snippet.match}</strong>
                      {snippet.after}
                    </span>
                  )}
                </span>
                <span className="go" aria-hidden="true">
                  Open
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
