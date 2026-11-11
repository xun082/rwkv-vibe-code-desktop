import { test } from 'node:test';
import assert from 'node:assert/strict';
import { summarizePerformance, waitingStage } from '../src/lib/performance.ts';
const job = (overrides = {}) => ({ id: 'a', prompt: '', title: '', createdAt: 0, startedAt: 1000, status: 'running', raw: '中文字数与 token 数不同', html: '', stats: { outputTokens: 100, tps: 55 }, ...overrides });
test('aggregate TPS uses engine tokens over elapsed inference time, not characters or summed per-stream averages', () => {
  const summary = summarizePerformance([job(), job({ id: 'b', stats: { outputTokens: 80, tps: 44 } })], 3000);
  assert.equal(summary.outputTokens, 180); assert.equal(summary.tps, 90); assert.equal(summary.elapsedMs, 3000);
});
test('completion/cancellation freeze the denominator and retain the last reported token counts', () => {
  const jobs = [job({ status:'done', endedAt:3000 }), job({ id:'b',status:'cancelled', endedAt:4000, stats:{outputTokens:50,tps:20} })];
  assert.equal(summarizePerformance(jobs,100000).tps,50);
  assert.equal(summarizePerformance(jobs,100000).elapsedMs,4000);
  assert.equal(summarizePerformance([],100000).tps,null);
});
test('waiting state distinguishes prefill, CSS-only output and body content', () => {
  assert.match(waitingStage('',0),/首个 token/);
  assert.match(waitingStage('<html><head><style>body{',40),/样式/);
  assert.match(waitingStage('<html><body><h1>正文',70),/正文/);
});

test('completed jobs use backend elapsed time if a terminal timestamp was not available', () => {
  const j = job({status:'done', stats:{outputTokens:100,tps:40,elapsedMs:2500}});
  const m = summarizePerformance([j],999999);
  assert.equal(m.inferenceMs,2500); assert.equal(m.tps,40); assert.equal(m.elapsedMs,3500);
});
