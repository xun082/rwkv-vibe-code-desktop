import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildPromptBatch } from '../src/lib/batch-prompts.ts';

test('one batch carries different multiline prompts without splitting a requirement into requests', () => {
  const input = '咖啡店官网\n三款咖啡与营业时间\n---\n精品酒店官网\n四种客房与预订信息';
  assert.deepEqual(buildPromptBatch(input, 1), ['咖啡店官网\n三款咖啡与营业时间', '精品酒店官网\n四种客房与预订信息']);
  assert.deepEqual(buildPromptBatch('六款饮品与价格', 2), ['六款饮品与价格', '六款饮品与价格']);
  assert.deepEqual(buildPromptBatch('单页网站', 1), ['单页网站']);
  assert.deepEqual(buildPromptBatch('A\n---\nB\n---\nC\n---\nD', 1), ['A', 'B', 'C', 'D']);
  assert.equal(buildPromptBatch('完整的单页需求', 4).length, 4);
  assert.equal(buildPromptBatch('完整的单页需求', 3).length, 3);
  assert.throws(() => buildPromptBatch('A\n---\nB\n---\nC\n---\nD\n---\nE', 4), /最多 4/);
  assert.throws(() => buildPromptBatch('a'.repeat(2001), 1), /2000/);
});
