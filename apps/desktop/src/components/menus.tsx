/**
 * Dropdown action menu buttons built on shared overlay primitives.
 *
 * `MenuButton` opens a dropdown menu next to the control that owns it.
 */

import { useId, useRef, useState } from 'react';
import type { MouseEvent, ReactNode, RefObject } from 'react';
import { Menu, Popover } from '/kit/overlay-primitives';
import { Button } from './button';
import type { ButtonSize, ButtonVariant } from './button';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

/** Receives a callback that closes the containing menu. */
export type MenuContent = (close: () => void) => ReactNode;

export interface MenuItemProps {
  /**
   * Why the action does not apply here. An item with a reason keeps its place
   * in the menu but does nothing: it is marked `aria-disabled` rather than
   * `disabled`, so the keyboard can reach it and read the reason out. A
   * natively disabled control is skipped by the roving selector, and
   * WKWebView suppresses hover — and so the `title` — on one.
   */
  reason?: string;
  /** The design's destructive item. */
  danger?: boolean;
  /** The glyph before the label. */
  icon?: FoksIconName;
  /** What the item says when it applies. */
  title?: string;
  onClick?: () => void;
  children: ReactNode;
}

export function MenuItem({
  reason,
  danger = false,
  icon,
  title,
  onClick,
  children,
}: MenuItemProps): ReactNode {
  const inert = reason !== undefined;
  const descriptionId = useId();
  return (
    <>
      <button
        type="button"
        className={danger ? 'danger' : undefined}
        aria-disabled={inert ? true : undefined}
        aria-describedby={inert ? descriptionId : undefined}
        tabIndex={inert ? -1 : undefined}
        title={reason ?? title}
        onClick={inert ? undefined : onClick}
      >
        {icon ? <Icon name={icon} /> : null}
        {children}
      </button>
      {reason ? (
        <span id={descriptionId} className="offscreen">
          {reason}
        </span>
      ) : null}
    </>
  );
}

function AnchoredMenu({
  anchorRef,
  label,
  align,
  close,
  children,
}: {
  anchorRef: RefObject<HTMLElement | null>;
  label: string;
  align: 'start' | 'end';
  close: () => void;
  children: MenuContent;
}): ReactNode {
  return (
    <Popover
      anchorRef={anchorRef}
      className="menu-portal"
      align={align}
      gap={4}
      onClose={close}
    >
      <Menu
        className="menu"
        anchorRef={anchorRef}
        onClose={close}
        aria-label={label}
        onClick={(event: MouseEvent<HTMLDivElement>) => {
          // Selecting an enabled menu item closes the menu. An item that does
          // not apply — marked `aria-disabled` so it keeps its place in the
          // keyboard order and can say why — does not close it.
          const target = event.target instanceof Element ? event.target : null;
          if (
            target?.closest(
              'button:not([disabled]):not([aria-disabled="true"])',
            )
          )
            close();
        }}
      >
        {children(close)}
      </Menu>
    </Popover>
  );
}

export interface MenuButtonProps {
  /** The trigger's label. */
  label: ReactNode;
  /** Accessible name describing the menu's choices. */
  menuLabel: string;
  align?: 'start' | 'end';
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: FoksIconName;
  /** Drawn after the label — the design's chevron. Pass `null` for none. */
  trailingIcon?: FoksIconName | null;
  className?: string;
  disabled?: boolean;
  title?: string;
  /** Accessible name when `label` is empty — icon-only triggers. */
  'aria-label'?: string;
  /** Open on the first paint — the acceptance scene for a dropdown. */
  defaultOpen?: boolean;
  /** The menu's items, given a closer to call when one is taken. */
  children: MenuContent;
}

export function MenuButton({
  label,
  menuLabel,
  align = 'end',
  variant = 'plain',
  size = 'md',
  icon,
  trailingIcon = 'chevronDown',
  className,
  disabled = false,
  title,
  'aria-label': ariaLabel,
  defaultOpen = false,
  children,
}: MenuButtonProps): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(defaultOpen && !disabled);
  const close = (): void => {
    setOpen(false);
  };
  const emptyLabel = typeof label === 'string' && label.trim() === '';
  return (
    <span className={['menuwrap', className ?? ''].filter(Boolean).join(' ')}>
      <Button
        ref={anchorRef}
        variant={variant}
        size={size}
        icon={icon}
        disabled={disabled}
        title={title}
        aria-label={
          ariaLabel ?? (emptyLabel ? (title ?? menuLabel) : undefined)
        }
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          if (disabled) return;
          setOpen((was) => !was);
        }}
      >
        {label}
        {trailingIcon != null ? (
          <Icon name={trailingIcon} className="chevron" />
        ) : null}
      </Button>
      {open ? (
        <AnchoredMenu
          anchorRef={anchorRef}
          label={menuLabel}
          align={align}
          close={close}
        >
          {children}
        </AnchoredMenu>
      ) : null}
    </span>
  );
}
