/**
 * Semantic icon registry for the FOKS shell.
 *
 * Product code names the meaning of an icon while this module owns the
 * corresponding Lucide glyph. Keeping the mapping here prevents screens from
 * depending on library export names or maintaining SVG path data themselves.
 */

import type { IconNode } from 'lucide';
import ArrowLeft from 'lucide/dist/esm/icons/arrow-left.mjs';
import Check from 'lucide/dist/esm/icons/check.mjs';
import ChevronDown from 'lucide/dist/esm/icons/chevron-down.mjs';
import CircleAlert from 'lucide/dist/esm/icons/circle-alert.mjs';
import CircleCheck from 'lucide/dist/esm/icons/circle-check.mjs';
import Copy from 'lucide/dist/esm/icons/copy.mjs';
import DoorOpen from 'lucide/dist/esm/icons/door-open.mjs';
import Download from 'lucide/dist/esm/icons/download.mjs';
import Ellipsis from 'lucide/dist/esm/icons/ellipsis.mjs';
import ExternalLink from 'lucide/dist/esm/icons/external-link.mjs';
import Eye from 'lucide/dist/esm/icons/eye.mjs';
import EyeOff from 'lucide/dist/esm/icons/eye-off.mjs';
import File from 'lucide/dist/esm/icons/file.mjs';
import Flag from 'lucide/dist/esm/icons/flag.mjs';
import Folder from 'lucide/dist/esm/icons/folder.mjs';
import Info from 'lucide/dist/esm/icons/info.mjs';
import KeyRound from 'lucide/dist/esm/icons/key-round.mjs';
import Laptop from 'lucide/dist/esm/icons/laptop.mjs';
import LayoutGrid from 'lucide/dist/esm/icons/layout-grid.mjs';
import List from 'lucide/dist/esm/icons/list.mjs';
import MessageSquare from 'lucide/dist/esm/icons/message-square.mjs';
import PanelLeftClose from 'lucide/dist/esm/icons/panel-left-close.mjs';
import PanelLeftOpen from 'lucide/dist/esm/icons/panel-left-open.mjs';
import Pencil from 'lucide/dist/esm/icons/pencil.mjs';
import Plug from 'lucide/dist/esm/icons/plug.mjs';
import Plus from 'lucide/dist/esm/icons/plus.mjs';
import RefreshCw from 'lucide/dist/esm/icons/refresh-cw.mjs';
import Search from 'lucide/dist/esm/icons/search.mjs';
import Send from 'lucide/dist/esm/icons/send.mjs';
import Server from 'lucide/dist/esm/icons/server.mjs';
import Settings from 'lucide/dist/esm/icons/settings.mjs';
import ShieldCheck from 'lucide/dist/esm/icons/shield-check.mjs';
import Terminal from 'lucide/dist/esm/icons/terminal.mjs';
import Trash from 'lucide/dist/esm/icons/trash.mjs';
import Upload from 'lucide/dist/esm/icons/upload.mjs';
import User from 'lucide/dist/esm/icons/user.mjs';
import Users from 'lucide/dist/esm/icons/users.mjs';
import Vault from 'lucide/dist/esm/icons/vault.mjs';
import X from 'lucide/dist/esm/icons/x.mjs';
import type { FoksIconName } from './icon-name';

export const FOKS_ICONS = {
  key: KeyRound,
  terminal: Terminal,
  laptop: Laptop,
  file: File,
  users: Users,
  user: User,
  vault: Vault,
  search: Search,
  grid: LayoutGrid,
  chevronDown: ChevronDown,
  info: Info,
  download: Download,
  upload: Upload,
  list: List,
  copy: Copy,
  trash: Trash,
  eye: Eye,
  eyeOff: EyeOff,
  plus: Plus,
  folder: Folder,
  close: X,
  settings: Settings,
  server: Server,
  check: Check,
  circleCheck: CircleCheck,
  pencil: Pencil,
  arrowLeft: ArrowLeft,
  shield: ShieldCheck,
  alert: CircleAlert,
  refresh: RefreshCw,
  ellipsis: Ellipsis,
  externalLink: ExternalLink,
  flag: Flag,
  door: DoorOpen,
  chat: MessageSquare,
  send: Send,
  panelLeftClose: PanelLeftClose,
  panelLeftOpen: PanelLeftOpen,
  plug: Plug,
} as const satisfies Record<FoksIconName, IconNode>;

export type { FoksIconName } from './icon-name';
