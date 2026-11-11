import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PreviewQueue } from '../src/lib/preview-queue.ts';

test('a busy preview skips stale snapshots and sends the newest content after acknowledgement', () => {
  const queue = new PreviewQueue();
  assert.equal(queue.take('first'), null);
  queue.markReady();
  const first = queue.take('first');
  assert.ok(first);
  assert.equal(queue.take('second'), null);
  assert.equal(queue.take('third'), null);
  assert.equal(queue.acknowledge(first.sequence - 1), false);
  assert.equal(queue.take('fourth'), null);
  assert.equal(queue.acknowledge(first.sequence), true);
  const latest = queue.take('fourth');
  assert.equal(latest.html, 'fourth');
  queue.acknowledge(latest.sequence);
  assert.equal(queue.take('fourth'), null);
});

test('a new document ignores old acknowledgements, and a failed patch can retry', () => {
  const queue = new PreviewQueue(); queue.markReady();
  const previous = queue.take('old document');
  queue.reset(); queue.markReady();
  const current = queue.take('new document');
  assert.equal(queue.acknowledge(previous.sequence), false);
  assert.equal(queue.take('newer'), null);
  queue.reject(current.sequence);
  assert.ok(queue.take('new document'));
});
