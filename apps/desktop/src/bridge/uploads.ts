import {
  array,
  integer,
  nullableInteger,
  nullableString,
  record,
  string,
} from './validation';
export type FileUpload = {
  uploadId: string;
  state: string;
  preparedChunks: number;
  acknowledgedChunks: number;
  plaintextBytes: number | null;
  reservedBytes: number;
  nodeId: string | null;
  version: number | null;
};
export type FileUploads = {
  uploads: FileUpload[];
  capacity: { maxReservedBytes: number; maxPending: number };
};
export function decodeFileUpload(value: unknown): FileUpload {
  const r = record(value, 'upload');
  const uploadId = string(r.upload_id, 'upload ID');
  const state = string(r.state, 'upload state');
  if (!/^[0-9a-f]{32}$/.test(uploadId)) throw new Error('Invalid upload ID');
  if (
    ![
      'awaiting-source',
      'inline-untracked',
      'needs-source',
      'ready',
      'uploading',
      'content-complete',
      'publishing',
      'verified',
      'completed',
      'blocked',
      'cancelled',
    ].includes(state)
  )
    throw new Error('Invalid upload state');
  const preparedChunks = integer(r.prepared_chunks, 'prepared chunks');
  const acknowledgedChunks = integer(
    r.acknowledged_chunks,
    'acknowledged chunks',
  );
  if (preparedChunks > 256 || acknowledgedChunks > preparedChunks)
    throw new Error('Invalid upload progress');
  return {
    uploadId,
    state,
    preparedChunks,
    acknowledgedChunks,
    plaintextBytes: nullableInteger(r.plaintext_bytes, 'plaintext bytes'),
    reservedBytes: integer(r.reserved_bytes, 'reserved bytes'),
    nodeId: nullableString(r.node_id, 'node ID'),
    version: nullableInteger(r.version, 'version'),
  };
}
export function decodeFileUploads(value: unknown): FileUploads {
  const r = record(value, 'uploads');
  const capacity = record(r.capacity, 'upload capacity');
  const uploads = array(r.uploads, 'uploads', decodeFileUpload);
  if (uploads.length > 1024) throw new Error('Upload inventory exceeds limit');
  return {
    uploads,
    capacity: {
      maxReservedBytes: integer(
        capacity.max_reserved_bytes,
        'upload capacity bytes',
      ),
      maxPending: integer(capacity.max_pending, 'pending upload capacity'),
    },
  };
}
