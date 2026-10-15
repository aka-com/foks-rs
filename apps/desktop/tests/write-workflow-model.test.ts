/**
 * The pure write-workflow model behind the shell's write overlay: the paths
 * a dropped file takes, the sheet an edit conflict resumes in, and the
 * workflow a `?state=` scene or a command error opens. Loaded in node with
 * no DOM.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/mock/fixture';
import {
  DEFAULT_READ_ROLE,
  DEFAULT_WRITE_ROLE,
} from '../src/screens/item-roles';
import {
  conflictDraftWorkflow,
  droppedFileDraft,
  fileDropPath,
  initialWriteWorkflow,
  namedPath,
  workflowForError,
} from '../src/screens/write-workflow-model';

const PATH = '/logins/github.com';

function githubLogin() {
  const item = FIXTURE.items.find(
    (entry) => entry.store === 'acct:personal' && entry.path === PATH,
  );
  assert.ok(item);
  return item;
}

test('a dropped file lands in the open folder, else at the root', () => {
  assert.equal(fileDropPath('scan.pdf'), '/scan.pdf');
  assert.equal(fileDropPath('scan.pdf', '/'), '/scan.pdf');
  assert.equal(fileDropPath('scan.pdf', '/docs'), '/docs/scan.pdf');
  assert.equal(fileDropPath('scan.pdf', '/docs/'), '/docs/scan.pdf');
  assert.equal(namedPath('github.com', '/logins'), '/logins/github.com');
  assert.equal(namedPath('github.com', '/logins', '/work'), '/work/github.com');
});

test('a dropped file draft carries its path, its source and the default roles', () => {
  assert.deepEqual(droppedFileDraft('/docs/scan.pdf', '/tmp/scan.pdf'), {
    path: '/docs/scan.pdf',
    site: '',
    username: '',
    password: '',
    website: '',
    value: '',
    resourceName: '',
    sourcePath: '/tmp/scan.pdf',
    readRole: DEFAULT_READ_ROLE,
    writeRole: DEFAULT_WRITE_ROLE,
  });
});

test('a deleted login resumes as a Document when the Password sheet cannot hold every line', () => {
  const item = githubLogin();

  /** The kind and draft of the sheet a deleted item's edit resumes in. */
  const resume = (draft: string) => {
    const workflow = conflictDraftWorkflow(item, draft);
    assert.ok(workflow?.kind === 'new');
    return { itemKind: workflow.itemKind, draft: workflow.draft };
  };

  const login = resume('user: kept\npassword:  padded \nurl: \n');
  assert.equal(login.itemKind, 'Password');
  assert.equal(login.draft?.password, ' padded ');

  for (const draft of [
    'ssid: Cabin\npassword: x',
    'user: a\npassword: x\nrotate this every quarter',
    'password: first\npassword: second',
  ]) {
    const resumed = resume(draft);
    assert.equal(resumed.itemKind, 'Document', draft);
    assert.equal(resumed.draft?.value, draft);
  }
});

test('only a conflict error with an item opens the conflict sheet', () => {
  const item = githubLogin();
  const conflict = {
    code: 'conflict',
    message: 'The item changed.',
    retryable: false,
    fatal: false,
    ambiguous: false,
  };
  assert.deepEqual(workflowForError(conflict, item, 'draft'), {
    kind: 'conflict',
    item,
    draft: 'draft',
  });
  assert.equal(workflowForError(conflict), null);
  assert.equal(workflowForError(new Error('boom'), item), null);
});

test('a ?state= scene opens its write workflow', () => {
  assert.deepEqual(initialWriteWorkflow('?state=new'), {
    kind: 'new',
    itemKind: 'Password',
    storeId: 'team:household',
  });
  assert.deepEqual(initialWriteWorkflow('?state=exists'), {
    kind: 'exists',
    itemKind: 'Password',
    storeId: 'acct:personal',
    path: '/logins/github.com',
    initialFolder: '/logins',
  });
  const conflict = initialWriteWorkflow('?state=conflict', FIXTURE);
  assert.equal(conflict?.kind, 'conflict');
  assert.equal(initialWriteWorkflow('?state=conflict'), null);
  assert.equal(initialWriteWorkflow(''), null);
});
