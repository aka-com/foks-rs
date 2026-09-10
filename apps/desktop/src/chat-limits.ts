/** Shared with the Rust agent protocol; values are UTF-8 bytes or row counts. */
import limits from '../../../crates/foks-agent-proto/chat-limits.json';

export const {
  CHAT_TEXT_BYTES,
  CHAT_PAGE_ROWS,
  CHAT_CHANNEL_ROWS,
  CHAT_NAME_BYTES,
  CHAT_LABEL_BYTES,
  CHAT_MISSING_PREDECESSORS,
  CHAT_PENDING_ROWS,
  CHAT_HISTORY_ROWS,
  CHAT_HISTORY_BYTES,
} = limits;
