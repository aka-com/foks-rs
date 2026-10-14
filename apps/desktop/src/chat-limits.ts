/**
 * Shared with the Rust agent protocol; values are UTF-8 bytes, character counts
 * or row counts. `*_CHARS` bounds count Unicode scalar values. The channel name
 * and description bounds are the ones `ChatLimits`
 * (`crates/foks-client-db/src/chat_limits.rs`) admits a channel on, and
 * `crates/foks-agent/src/chat.rs` asserts the two sides agree.
 * `CHAT_NOTIFICATION_TEXT_CHARS` is the length the agent cuts each
 * notification-history text to.
 */
import limits from '../../../crates/foks-agent-proto/chat-limits.json';

export const {
  CHAT_TEXT_BYTES,
  CHAT_SNIPPET_BYTES,
  CHAT_NOTIFICATION_TEXT_CHARS,
  CHAT_PAGE_ROWS,
  CHAT_CHANNEL_ROWS,
  CHAT_NAME_BYTES,
  CHAT_NAME_MIN_CHARS,
  CHAT_NAME_MAX_CHARS,
  CHAT_DESCRIPTION_BYTES,
  CHAT_DESCRIPTION_MIN_CHARS,
  CHAT_DESCRIPTION_MAX_CHARS,
  CHAT_LABEL_BYTES,
  CHAT_MISSING_PREDECESSORS,
  CHAT_PENDING_ROWS,
  CHAT_HISTORY_ROWS,
  CHAT_HISTORY_BYTES,
  CHAT_INBOX_ROWS,
  CHAT_POLL_MILLISECONDS,
} = limits;
