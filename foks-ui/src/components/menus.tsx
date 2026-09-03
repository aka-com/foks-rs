/**
 * Buttons that open a menu, over the kit's overlay primitives.
 *
 * `ui/kit/overlay-primitives.tsx` already owns the hard parts — anchored
 * placement that stays inside the viewport, roving focus, Escape, dismissal
 * on an outside pointer-down, and returning focus to the trigger — so this
 * file adds the design's shapes and nothing else. Rewriting any of that here
 * would give FOKS a second, worse copy of a tested primitive.
 *
 * The one adaptation is where the menu lives. The mock draws `.menu`
 * absolutely inside `.menuwrap`; a portaled menu escapes the scrolling
 * surfaces instead, so the portal wrap is fixed and `.menu` inside it is
 * static (`src/styles/app.css`).
 */

import { useRef, useState } from 'react';
import type { MouseEvent, ReactNode, RefObject } from 'react';
import { Menu, Popover } from '/kit/overlay-primitives';
import { Button } from './button';
import type { ButtonSize, ButtonVariant } from './button';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

/** Called with a closer, so an item can dismiss the menu it lives in. */
export type MenuContent = (close: () => void) => ReactNode;

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
          // A menu closes when one of its items is taken. A disabled item is
          // not a choice, so it leaves the menu open.
          const target = event.target instanceof Element ? event.target : null;
          if (target?.closest('button:not([disabled])')) close();
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
  /** What the menu chooses between, for the screen reader. */
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
  trailingIcon = 'chev',
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

export interface SplitButtonProps {
  /** The primary action's label. */
  label: ReactNode;
  icon?: FoksIconName;
  /** What the chevron's menu chooses between. */
  menuLabel: string;
  /** The primary action itself. Omit it and the whole control opens the menu. */
  onClick?: () => void;
  /** The menu's items, given a closer to call when one is taken. */
  children: MenuContent;
}

export function SplitButton({
  label,
  icon,
  menuLabel,
  onClick,
  children,
}: SplitButtonProps): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const toggle = (): void => {
    setOpen((was) => !was);
  };
  return (
    <span className="menuwrap">
      <span className="split">
        <Button
          ref={anchorRef}
          variant="primary"
          icon={icon}
          aria-haspopup={onClick ? undefined : 'menu'}
          aria-expanded={onClick ? undefined : open}
          onClick={onClick ?? toggle}
        >
          {label}
        </Button>
        <Button
          variant="primary"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={menuLabel}
          onClick={toggle}
        >
          <Icon name="chev" className="chevron" />
        </Button>
      </span>
      {open ? (
        <AnchoredMenu
          anchorRef={anchorRef}
          label={menuLabel}
          align="start"
          close={() => {
            setOpen(false);
          }}
        >
          {children}
        </AnchoredMenu>
      ) : null}
    </span>
  );
}
