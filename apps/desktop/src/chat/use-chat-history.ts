import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type {
  ChatAction,
  ChatChannel,
  ChatMessage,
  ChatReply,
} from '../chat-contract';
import { failure } from './actions';
import { CHAT_HISTORY_ROWS, CHAT_HISTORY_BYTES } from '../chat-limits';

function ordered(rows: ChatMessage[]): ChatMessage[] {
  return [...rows].sort((a, b) =>
    BigInt(a.sequence) < BigInt(b.sequence) ? -1 : 1,
  );
}
function merge(
  existing: ChatMessage[],
  incoming: ChatMessage[],
): ChatMessage[] {
  const rows = new Map(existing.map((m) => [m.id, m]));
  for (const message of incoming) {
    const old = rows.get(message.id);
    if (old && JSON.stringify(old) !== JSON.stringify(message))
      throw new Error(
        'The message changed while refreshing. Reopen this conversation.',
      );
    rows.set(message.id, message);
  }
  return ordered([...rows.values()]);
}
/** Manages history loading, page verification, and scroll position for the active channel. */
export function useChatHistory(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  revision = 0,
) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [before, setBefore] = useState<string | null>(null);
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const active = useRef(false);
  const reading = useRef(false);
  const scroller = useRef<HTMLDivElement>(null);
  const currentMessages = useRef<ChatMessage[]>([]);
  const verification = useRef(new Map<string, boolean>());
  const followBottom = useRef(true);
  const scroll = useRef<{ height: number; top: number } | null>(null);
  const seenRevision = useRef(revision);
  const onScroll = useCallback(() => {
    const element = scroller.current;
    if (!element) return;
    setAtBottom(
      element.scrollHeight - element.scrollTop - element.clientHeight < 80,
    );
  }, []);
  const load = useCallback(
    async (older: string | null = null) => {
      if (reading.current) return;
      reading.current = true;
      setBusy(true);
      setError('');
      followBottom.current =
        !older &&
        (!scroller.current ||
          scroller.current.scrollHeight -
            scroller.current.scrollTop -
            scroller.current.clientHeight <
            80);
      if (older) setAtBottom(false);
      if (older && scroller.current)
        scroll.current = {
          height: scroller.current.scrollHeight,
          top: scroller.current.scrollTop,
        };
      try {
        const reply = await request({
          action: 'history',
          channel: channel.id,
          before: older,
        });
        if (!active.current) return;
        if (reply.result.kind === 'history') {
          const page = reply.result;
          // A latest-page refresh can jump past the retained window. Start a new
          // window so its older cursor can reach every message in between.
          const last = currentMessages.current.at(-1);
          const first = ordered(page.messages)[0];
          const reset =
            !older &&
            !!last &&
            !!first &&
            BigInt(first.sequence) > BigInt(last.sequence) + 1n;
          const rows = merge(
            reset ? [] : currentMessages.current,
            page.messages,
          );
          if (
            rows.length > CHAT_HISTORY_ROWS ||
            rows.reduce(
              (n, m) =>
                n +
                (m.content.kind === 'text'
                  ? new TextEncoder().encode(m.content.text).length
                  : 0),
              0,
            ) > CHAT_HISTORY_BYTES
          )
            throw new Error(
              'This conversation view reached its history limit. Reopen it to load the newest messages.',
            );
          currentMessages.current = rows;
          setMessages(rows);
          setBefore((old) =>
            reset || older || old === null ? page.before : old,
          );
          if (reset) verification.current.clear();
          for (const message of page.messages)
            verification.current.set(
              message.id,
              page.missing_predecessors.length > 0,
            );
          setMissing([...verification.current.values()].some(Boolean));
        }
      } catch (e) {
        if (active.current) setError(failure(e));
      } finally {
        reading.current = false;
        if (active.current) setBusy(false);
      }
    },
    [request, channel.id],
  );
  useEffect(() => {
    active.current = true;
    if (channel.readable) void load();
    return () => {
      active.current = false;
    };
  }, [load, channel.readable]);
  useEffect(() => {
    if (revision === seenRevision.current) return;
    seenRevision.current = revision;
    if (active.current && channel.readable) void load();
  }, [channel.readable, load, revision]);
  useLayoutEffect(() => {
    if (scroll.current && scroller.current) {
      scroller.current.scrollTop =
        scroll.current.top +
        scroller.current.scrollHeight -
        scroll.current.height;
      scroll.current = null;
    } else if (followBottom.current && scroller.current) {
      scroller.current.scrollTop = scroller.current.scrollHeight;
    }
    onScroll();
  }, [messages, onScroll]);
  return {
    messages,
    before,
    missing,
    error,
    setError,
    busy,
    load,
    scroller,
    active,
    atBottom,
    onScroll,
  };
}
