import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeFileUpload, decodeFileUploads } from '../src/bridge/uploads';
const upload = {
  upload_id: '01'.repeat(16),
  state: 'uploading',
  prepared_chunks: 2,
  acknowledged_chunks: 1,
  plaintext_bytes: 4194427,
  reserved_bytes: 9000000,
  node_id: null,
  version: null,
};
test('durable upload progress preserves a stable handle without source paths', () => {
  const result = decodeFileUpload(upload);
  assert.equal(result.uploadId, upload.upload_id);
  assert.equal(result.acknowledgedChunks, 1);
  assert.equal(result.preparedChunks, 2);
  assert.equal(
    decodeFileUploads({
      uploads: [upload],
      capacity: {
        max_reserved_bytes: 268435456,
        max_pending: 16,
        max_receipts: 8192,
        max_receipt_bytes: 16777216,
      },
    }).uploads.length,
    1,
  );
});
test('upload bridge rejects malformed identity and impossible progress', () => {
  for (const changed of [
    { upload_id: 'bad' },
    { acknowledged_chunks: 3 },
    { prepared_chunks: 257 },
    { state: 'success-ish' },
    { reserved_bytes: -1 },
  ])
    assert.throws(() => decodeFileUpload({ ...upload, ...changed }));
});

test('generation-bound handles and bounded summary pages preserve cursor identity', () => {
  const id = `u1-${'ab'.repeat(16)}-0000000067000000-${'cd'.repeat(16)}`;
  assert.equal(decodeFileUpload({ ...upload, upload_id: id }).uploadId, id);
  const capacity = {
    max_reserved_bytes: 268435456,
    max_pending: 16,
    max_receipts: 8192,
    max_receipt_bytes: 16777216,
  };
  const page = decodeFileUploads({
    uploads: [upload],
    next: Array(32).fill(7),
    capacity,
  });
  assert.deepEqual(page.next, Array(32).fill(7));
  assert.equal(page.capacity.maxReceipts, 8192);
  assert.throws(() =>
    decodeFileUploads({ uploads: Array(129).fill(upload), capacity }),
  );
  assert.throws(() => decodeFileUploads({ uploads: [], next: [1], capacity }));
  assert.throws(() =>
    decodeFileUpload({ ...upload, upload_id: id.toUpperCase() }),
  );
});
