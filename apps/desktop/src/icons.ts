/**
 * Shared SVG icon definitions for the FOKS shell.
 * Stored as structural element tuples ([tag, attributes]) rendered by Icon components.
 */

/** Represents a single SVG child element definition as a tag and attribute record tuple. */
export type IconElement = readonly [
  tag: 'path' | 'circle' | 'rect',
  attrs: Readonly<Record<string, string | number>>,
];

export const FOKS_ICONS = {
  key: [
    ['circle', { cx: 8, cy: 14, r: 4 }],
    ['path', { d: 'M11 11l9-9M17 5l2.5 2.5M14.5 7.5 17 10' }],
  ],
  term: [
    ['rect', { x: 3, y: 5, width: 18, height: 14, rx: 2 }],
    ['path', { d: 'M7 9l3 3-3 3M12 15h5' }],
  ],
  file: [
    [
      'path',
      { d: 'M6 3h8l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z' },
    ],
    ['path', { d: 'M14 3v4h4' }],
  ],
  link: [
    ['path', { d: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1' }],
    ['path', { d: 'M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1' }],
  ],
  people: [
    ['circle', { cx: 9, cy: 8, r: 3.2 }],
    ['path', { d: 'M3 19c0-3.3 2.7-5.5 6-5.5s6 2.2 6 5.5' }],
    ['path', { d: 'M15.5 5.2a3.2 3.2 0 0 1 0 5.6' }],
    ['path', { d: 'M17 13.6c2.4.5 4 2.5 4 5.4' }],
  ],
  person: [
    ['circle', { cx: 12, cy: 8, r: 3.6 }],
    ['path', { d: 'M5 20c.6-4 3.4-6 7-6s6.4 2 7 6' }],
  ],
  vault: [
    ['rect', { x: 3, y: 4, width: 18, height: 16, rx: 2.5 }],
    ['circle', { cx: 12, cy: 12, r: 3.5 }],
    ['path', { d: 'M12 8.5v1.5M12 14v1.5M8.5 12H10M14 12h1.5' }],
  ],
  search: [
    ['circle', { cx: 11, cy: 11, r: 6.5 }],
    ['path', { d: 'M16 16l4.5 4.5' }],
  ],
  grid: [
    ['rect', { x: 4, y: 4, width: 6.5, height: 6.5, rx: 1.5 }],
    ['rect', { x: 13.5, y: 4, width: 6.5, height: 6.5, rx: 1.5 }],
    ['rect', { x: 4, y: 13.5, width: 6.5, height: 6.5, rx: 1.5 }],
    ['rect', { x: 13.5, y: 13.5, width: 6.5, height: 6.5, rx: 1.5 }],
  ],
  list: [
    ['path', { d: 'M9 6h11M9 12h11M9 18h11' }],
    ['circle', { cx: 5, cy: 6, r: 1, fill: 'currentColor' }],
    ['circle', { cx: 5, cy: 12, r: 1, fill: 'currentColor' }],
    ['circle', { cx: 5, cy: 18, r: 1, fill: 'currentColor' }],
  ],
  chev: [['path', { d: 'M6 9l6 6 6-6' }]],
  info: [
    ['circle', { cx: 12, cy: 12, r: 8.5 }],
    ['path', { d: 'M12 11v5M12 8h.01' }],
  ],
  download: [['path', { d: 'M12 4v11M7 10l5 5 5-5M4 20h16' }]],
  copy: [
    ['rect', { x: 9, y: 9, width: 11, height: 11, rx: 2 }],
    ['path', { d: 'M5 15V5a1 1 0 0 1 1-1h10' }],
  ],
  path: [
    ['circle', { cx: 6, cy: 6, r: 2 }],
    ['circle', { cx: 18, cy: 18, r: 2 }],
    ['path', { d: 'M8 6h5a4 4 0 0 1 0 8h-2a4 4 0 0 0 0 4h5' }],
  ],
  trash: [['path', { d: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13' }]],
  eye: [
    ['path', { d: 'M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z' }],
    ['circle', { cx: 12, cy: 12, r: 3 }],
  ],
  eyeoff: [
    [
      'path',
      {
        d: 'M3 3l18 18M10.6 10.6a2 2 0 0 0 2.8 2.8M6.6 6.6C4 8.2 2 12 2 12s3.5 6 10 6c1.7 0 3.2-.4 4.4-1M9.9 6.2C10.5 6.1 11.2 6 12 6c6.5 0 10 6 10 6s-.8 1.4-2.3 2.9',
      },
    ],
  ],
  bell: [['path', { d: 'M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 21h4' }]],
  plus: [['path', { d: 'M12 5v14M5 12h14' }]],
  arrow: [['path', { d: 'M5 12h14M13 6l6 6-6 6' }]],
  arrowUpRight: [['path', { d: 'M6 18L18 6M6 6h12v12' }]],
  folder: [
    [
      'path',
      {
        d: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
      },
    ],
  ],
  sortName: [
    ['path', { d: 'M4 6h9M4 12h7M4 18h5' }],
    ['path', { d: 'M17 6v12M14 15l3 3 3-3' }],
  ],
  sortKind: [
    ['circle', { cx: 7.5, cy: 7.5, r: 3.5 }],
    ['rect', { x: 13, y: 4, width: 7, height: 7, rx: 1.5 }],
    ['path', { d: 'M7.5 13.5l3.5 6.5h-7z' }],
    ['path', { d: 'M13 20l3.5-6.5 3.5 6.5z' }],
  ],
  sortGroup: [
    ['path', { d: 'M12 4l8 4-8 4-8-4z' }],
    ['path', { d: 'M4 12l8 4 8-4' }],
    ['path', { d: 'M4 16l8 4 8-4' }],
  ],
  sortTime: [
    ['circle', { cx: 12, cy: 12, r: 8.5 }],
    ['path', { d: 'M12 7.5V12l3 2' }],
  ],
  x: [['path', { d: 'M6 6l12 12M18 6L6 18' }]],
  gear: [
    [
      'path',
      {
        d: 'M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.3a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4A2 2 0 0 0 4 9.9l.2.1a2 2 0 0 1 1 1.7v.5a2 2 0 0 1-1 1.8l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.3a2 2 0 0 1 1 1.7v.2a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.3a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.8v-.5a2 2 0 0 1 1-1.7l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.3a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2z',
      },
    ],
    ['circle', { cx: 12, cy: 12, r: 3 }],
  ],
  server: [
    ['rect', { x: 3, y: 4, width: 18, height: 6, rx: 1.5 }],
    ['rect', { x: 3, y: 14, width: 18, height: 6, rx: 1.5 }],
    ['path', { d: 'M7 7h.01M7 17h.01' }],
  ],
  check: [['path', { d: 'M5 12l5 5 9-10' }]],
  pencil: [
    ['path', { d: 'M4 20l4.5-1L19 8.5a2 2 0 0 0-3-3L5.5 16z' }],
    ['path', { d: 'M14 7l3 3' }],
  ],
  back: [['path', { d: 'M15 5l-7 7 7 7' }]],
  shield: [
    ['path', { d: 'M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z' }],
    ['path', { d: 'M9 12l2 2 4-4' }],
  ],
  alert: [
    ['circle', { cx: 12, cy: 12, r: 8.5 }],
    ['path', { d: 'M12 8v5M12 16h.01' }],
  ],
  minus: [['path', { d: 'M5 12h14' }]],
  again: [
    ['path', { d: 'M20 12a8 8 0 1 1-2.6-5.9' }],
    ['path', { d: 'M20 4v4h-4' }],
  ],
  more: [
    ['circle', { cx: 5, cy: 12, r: 1.2, fill: 'currentColor' }],
    ['circle', { cx: 12, cy: 12, r: 1.2, fill: 'currentColor' }],
    ['circle', { cx: 19, cy: 12, r: 1.2, fill: 'currentColor' }],
  ],
  out: [
    [
      'path',
      {
        d: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5',
      },
    ],
  ],
  mail: [
    ['rect', { x: 3, y: 5, width: 18, height: 14, rx: 2 }],
    ['path', { d: 'M3 7l9 6 9-6' }],
  ],
  flag: [['path', { d: 'M5 21V4h13l-2.5 4L18 12H5' }]],
  plug: [['path', { d: 'M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4' }]],
  door: [
    ['path', { d: 'M5 21V3h9v18M14 21h5M3 21h2' }],
    ['circle', { cx: 11.5, cy: 12, r: 1, fill: 'currentColor' }],
  ],
} as const satisfies Record<string, readonly IconElement[]>;

/** Every icon name the shell can draw. */
export type FoksIconName = keyof typeof FOKS_ICONS;

/** The names, in the order `shell.js` declares them. */
export const FOKS_ICON_NAMES = Object.keys(FOKS_ICONS) as FoksIconName[];
