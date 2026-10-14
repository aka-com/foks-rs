import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Button, Icon } from '../components';
import { Popover } from '/kit/overlay-primitives';
import { deliveryNeedsAttention, operationDelivery } from './delivery';
import type { TrackedOperation } from './operations';
import { PendingRow } from './pending-row';

/**
 * The team's saved work that the open thread does not draw, as a chip in the
 * conversation header: channel creations, and messages in other channels or
 * in a channel the reader can no longer read or that is stopped. The chip
 * counts the operations whose state leaves work for the reader
 * (`deliveryNeedsAttention`) and opens a popover that lists every one of them
 * with its requests and, for a message in a channel the reader can open, Jump.
 * Once the last of them is accounted for, the chip says so instead of
 * disappearing without a word, and the popover closes.
 */
export function UnsentChip({
  storeId,
  operations,
  recovered,
  teamName,
  channelName,
  canOpen,
  onJump,
  onChange,
}: {
  storeId: string;
  operations: readonly TrackedOperation[];
  /** Whether this conversation has listed saved work since it opened. */
  recovered: boolean;
  teamName?: string;
  /** The title of a listed channel. */
  channelName: (channel: string) => string | undefined;
  /** Whether a message's channel can be opened from here. */
  canOpen: (channel: string) => boolean;
  onJump: (channel: string) => void;
  onChange: () => void;
}): ReactNode {
  const [open, setOpen] = useState(false);
  const [refocus, setRefocus] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const settled = useRef<HTMLSpanElement>(null);
  // Resolving the last operation from the popover empties it, and it unmounts
  // without `onClose`. Close it here so the next operation does not reopen it
  // and take focus, and move focus from the removed popover to the status
  // that replaces the chip rather than leaving it on the document body.
  if (operations.length === 0 && open) {
    setOpen(false);
    setRefocus(true);
  }
  useEffect(() => {
    if (!refocus) return;
    setRefocus(false);
    const active = document.activeElement;
    if (!active || active === document.body)
      settled.current?.focus({ preventScroll: true });
  }, [refocus]);
  if (operations.length === 0) {
    if (!recovered) return null;
    return (
      <span
        ref={settled}
        tabIndex={-1}
        className="chip ok chat-unsent-chip"
        role="status"
        title="Everything saved on this device has been accounted for."
      >
        <Icon name="check" size={12} /> All caught up
        <span className="offscreen">
          . Everything saved on this device has been accounted for.
        </span>
      </span>
    );
  }
  const count = operations.filter((op) =>
    deliveryNeedsAttention(operationDelivery(op)),
  ).length;
  const label = count > 0 ? `${count} unsent` : 'Saved work';
  return (
    <>
      <Button
        ref={anchor}
        size="sm"
        variant="quiet"
        className={`chat-unsent-chip${count > 0 ? ' warn' : ''}`}
        icon="alert"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
      >
        {label}
      </Button>
      {open && (
        <UnsentPopover
          anchor={anchor}
          storeId={storeId}
          operations={operations}
          teamName={teamName}
          channelName={channelName}
          canOpen={canOpen}
          onJump={(channel) => {
            setOpen(false);
            onJump(channel);
          }}
          onChange={onChange}
          onClose={(restore) => {
            setOpen(false);
            if (restore) anchor.current?.focus({ preventScroll: true });
          }}
        />
      )}
    </>
  );
}

function UnsentPopover({
  anchor,
  storeId,
  operations,
  teamName,
  channelName,
  canOpen,
  onJump,
  onChange,
  onClose,
}: {
  anchor: RefObject<HTMLButtonElement | null>;
  storeId: string;
  operations: readonly TrackedOperation[];
  teamName?: string;
  channelName: (channel: string) => string | undefined;
  canOpen: (channel: string) => boolean;
  onJump: (channel: string) => void;
  onChange: () => void;
  /** `restore` is set when focus should return to the chip. */
  onClose: (restore: boolean) => void;
}): ReactNode {
  const titleId = useId();
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    close.current?.focus({ preventScroll: true });
  }, []);
  return (
    <Popover
      anchorRef={anchor}
      className="menu-portal chat-details chat-unsent"
      align="end"
      onClose={() => onClose(false)}
    >
      <div role="dialog" aria-labelledby={titleId}>
        <div className="chat-details-head">
          <h3 id={titleId}>Unsent work{teamName ? ` in ${teamName}` : ''}</h3>
          <button
            type="button"
            ref={close}
            aria-label="Close unsent work"
            title="Close"
            onClick={() => onClose(true)}
          >
            <Icon name="close" />
          </button>
        </div>
        <p className="chat-unsent-note">
          Saved on this device and not finished. Nothing here is sent again
          unless you ask.
        </p>
        <div className="chat-unsent-list">
          {operations.map((op) => (
            <PendingRow
              key={op.id}
              storeId={storeId}
              operation={op}
              channelName={
                op.kind === 'send-message' ? channelName(op.channel) : undefined
              }
              teamName={teamName}
              onChange={onChange}
              onJump={
                op.kind === 'send-message' && canOpen(op.channel)
                  ? () => onJump(op.channel)
                  : undefined
              }
            />
          ))}
        </div>
      </div>
    </Popover>
  );
}
