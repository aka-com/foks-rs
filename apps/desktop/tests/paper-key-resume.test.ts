import assert from 'node:assert/strict';
import test from 'node:test';
import { PaperKeyResume } from '../src/screens/paper-key-resume';

const draft = { phrase: 'test phrase', alias: 'travel' };

test('a concealed paper key expires after two minutes without extending on repeated blur', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const resume = new PaperKeyResume();
  resume.prepare(draft);
  resume.conceal();
  t.mock.timers.tick(119_999);
  assert.equal(resume.get()?.phrase, draft.phrase);
  resume.conceal();
  t.mock.timers.tick(1);
  assert.equal(resume.get(), null);
  assert.equal(resume.take(), null);
});

test('taking or dismissing a retained paper key removes it', () => {
  const resume = new PaperKeyResume();
  resume.prepare(draft);
  resume.conceal();
  assert.deepEqual(resume.take(), draft);
  assert.equal(resume.take(), null);
  resume.prepare(draft);
  resume.conceal();
  resume.forget();
  assert.equal(resume.get(), null);
});
