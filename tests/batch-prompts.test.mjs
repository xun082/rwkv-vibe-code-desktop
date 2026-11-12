import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildPromptBatch } from '../src/lib/batch-prompts.ts';

test('one batch carries different multiline prompts without splitting a requirement into requests', () => {
  const input = '咖啡店官网\n三款咖啡与营业时间\n---\n精品酒店官网\n四种客房与预订信息';
  assert.deepEqual(buildPromptBatch(input, 2), ['咖啡店官网\n三款咖啡与营业时间', '精品酒店官网\n四种客房与预订信息']);
  assert.deepEqual(buildPromptBatch('六款饮品与价格', 2), ['六款饮品与价格', '六款饮品与价格']);
  assert.equal(buildPromptBatch('完整的单页需求', 4).length, 4);
  assert.throws(() => buildPromptBatch('a'.repeat(2001), 2), /2000/);
});

test('batch accepts sixteen lanes and honors the selected limit for separate prompts', () => {
  const prompts = Array.from({ length: 16 }, (_, i) => `网页需求 ${i + 1}`);
  const input = prompts.join('\n---\n');
  assert.deepEqual(buildPromptBatch(input, 16), prompts);
  assert.equal(buildPromptBatch('单页网站', 16).length, 16);
  assert.throws(() => buildPromptBatch(input, 4), /当前为 4 路/);
  assert.throws(() => buildPromptBatch(`${input}\n---\n第十七条`, 16), /最多 16/);
  assert.throws(() => buildPromptBatch('单页网站', 1), /2–16/);
  assert.throws(() => buildPromptBatch('单页网站', 17), /2–16/);
  assert.throws(() => buildPromptBatch('单页网站', 5), /偶数/);
  assert.deepEqual(buildPromptBatch('A\n---\nB\n---\nC', 4), ['A', 'B', 'C']);
});
