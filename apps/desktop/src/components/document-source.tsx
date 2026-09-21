/**
 * The Text / File switch for a document's content.
 *
 * A document is either typed or brought in from disk, never both, so the form
 * shows one source at a time: a segmented control chooses, and the row it
 * selects is a value field or a drop target. Used wherever a document can be
 * created from either.
 */

import type { ReactNode } from 'react';
import { Field } from './field';
import { InsetRow } from './inset';
import { SegmentedControl } from './segmented-control';

export type DocumentSourceKind = 'text' | 'file';

export function DocumentSourceSwitch({
  value,
  onChange,
  disabled = false,
}: {
  value: DocumentSourceKind;
  onChange: (next: DocumentSourceKind) => void;
  disabled?: boolean;
}): ReactNode {
  return (
    <SegmentedControl<DocumentSourceKind>
      label="Document content"
      value={value}
      onChange={onChange}
      disabled={disabled}
      items={[
        {
          id: 'text',
          label: 'Text',
          title: 'Type or paste a note, key, or token',
        },
        { id: 'file', label: 'File', title: 'Bring in a file from disk' },
      ]}
    />
  );
}

export function DocumentSourceRow({
  source,
  value,
  onValue,
  placeholder = 'A note, key, or token',
  sourcePath,
  hovering = false,
}: {
  source: DocumentSourceKind;
  value: string;
  onValue: (next: string) => void;
  placeholder?: string;
  /** The file chosen or dropped so far, when the source is a file. */
  sourcePath: string | null;
  /** A file is being dragged over the form. */
  hovering?: boolean;
}): ReactNode {
  if (source === 'text') {
    return (
      <Field
        label="Value"
        value={value}
        onChange={onValue}
        placeholder={placeholder}
      />
    );
  }
  return (
    <InsetRow label="File">
      <span className={sourcePath ? 'mono' : 'dim'}>
        {sourcePath
          ? sourcePath.split(/[\\/]/).at(-1)
          : hovering
            ? 'Drop to use this file'
            : 'Drop a file here, or choose a file'}
      </span>
    </InsetRow>
  );
}
