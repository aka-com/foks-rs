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
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

/** Receives a callback that closes the containing menu. */
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
          // Selecting an enabled menu item closes the menu. Disabled items do
          // not close it.
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
  /** Accessible name describing the menu's choices. */
  menuLabel: string;
  align?: 'start' | 'end';
  icon?: FoksIconName;
  /** Drawn after the label — the design's chevron. */
  trailingIcon?: FoksIconName;
  className?: string;
  /** The menu's items, given a closer to call when one is taken. */
  children: MenuContent;
}

export function MenuButton({
  label,
  menuLabel,
  align = 'end',
  icon,
  trailingIcon = 'chev',
  className,
  children,
}: MenuButtonProps): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const close = (): void => {
    setOpen(false);
  };
  return (
    <span className={['menuwrap', className ?? ''].filter(Boolean).join(' ')}>
      <Button
        ref={anchorRef}
        icon={icon}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen((was) => !was);
        }}
      >
        {label}
        {trailingIcon ? <Icon name={trailingIcon} className="chevron" /> : null}
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
