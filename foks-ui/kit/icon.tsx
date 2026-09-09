import { createElement } from 'react';
import type { IconNode, SVGProps as LucideSvgProps } from 'lucide';
import type { ReactNode, SVGProps } from 'react';

export type IconDefinition =
  | { kind: 'lucide'; node: IconNode; size: number; attrs?: LucideSvgProps }
  | { kind: 'brand'; path: string; size: number };

const REACT_ATTR_NAMES: Record<string, string> = {
  class: 'className',
  'stroke-linecap': 'strokeLinecap',
  'stroke-linejoin': 'strokeLinejoin',
  'stroke-width': 'strokeWidth',
  tabindex: 'tabIndex',
};

function reactAttributes(
  attributes: Record<string, unknown>,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(attributes).map(([name, value]) => [
      REACT_ATTR_NAMES[name] ?? name,
      value,
    ]),
  );
}

function resolveIcon(
  icon?: IconDefinition | { default?: IconDefinition },
): IconDefinition | undefined {
  if (!icon || typeof icon !== 'object') return undefined;
  if ('kind' in icon) return icon;
  const nested = icon.default;
  if (nested && typeof nested === 'object' && 'kind' in nested) return nested;
  return undefined;
}

function lucideNode(
  icon: Extract<IconDefinition, { kind: 'lucide' }>,
): IconNode | null {
  const node = icon.node as IconNode | { default?: IconNode } | undefined;
  if (Array.isArray(node)) return node;
  if (node && Array.isArray(node.default)) return node.default;
  return null;
}

export function AppIcon({ icon }: { icon?: IconDefinition }): ReactNode {
  const resolved = resolveIcon(icon);
  if (!resolved) return null;
  const common: SVGProps<SVGSVGElement> = {
    xmlns: 'http://www.w3.org/2000/svg',
    viewBox: '0 0 24 24',
    width: resolved.size,
    height: resolved.size,
    'aria-hidden': true,
    focusable: false,
  };
  if (resolved.kind === 'brand') {
    return (
      <svg {...common} fill="currentColor">
        <path d={resolved.path} />
      </svg>
    );
  }
  const node = lucideNode(resolved);
  if (!node) return null;
  return (
    <svg
      {...common}
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      {...reactAttributes(resolved.attrs ?? {})}
    >
      {node.map(([tag, attributes], index) =>
        createElement(tag, {
          ...reactAttributes(attributes ?? {}),
          key: attributes?.key ?? index,
        }),
      )}
    </svg>
  );
}
