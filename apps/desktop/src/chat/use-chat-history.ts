import { normalizeCommandError } from '../bridge';
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
  ChatResult,
} from '../chat-contract';
import { failure } from './actions';
import type { HistoryWindow } from './conversation-model';

const EMPTY_MESSAGES: ChatMessage[] = [];

/** Manages history loading, page verification, and scroll position for the active channel. */
export function useChatHistory(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  revision = 0,
  onAccepted?: (
    page: Extract<ChatResult, { kind: 'history' }>,
    before: string | null,
  ) => void,
  history?: HistoryWindow | null,
  onFatal?: (channel: string) => void,
) {
  const accepted = history?.channel === channel.id ? history : null;
  const messages = accepted?.messages ?? EMPTY_MESSAGES;
  const before = accepted?.before ?? null;
  const missing = accepted
    ? [...accepted.verification.values()].some(Boolean)
    : false;
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const active = useRef(false);
  const reading = useRef(false);
  const again = useRef(false);
  const scroller = useRef<HTMLDivElement>(null);
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
      if (reading.current) {
        if (!older) again.current = true;
        return;
      }
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
          onAccepted?.(page, older);
        }
      } catch (e) {
        if (active.current) {
          if (normalizeCommandError(e).code === 'chat-channel-integrity')
            onFatal?.(channel.id);
          setError(failure(e));
        }
      } finally {
        reading.current = false;
        if (active.current) {
          setBusy(false);
          if (again.current) {
            again.current = false;
            void load();
          }
        }
      }
    },
    [request, channel.id, onAccepted, onFatal],
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
    atBottom,
    onScroll,
  };
}
