import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyGenerationEvent } from '../src/lib/generation-events.ts';
import { summarizePerformance } from '../src/lib/performance.ts';
test('a real metrics/delta/done sequence preserves counters and freezes completion time', () => {
  let j = { id:'test',title:'',prompt:'',createdAt:1000,status:'queued',raw:'',html:'' };
  j = applyGenerationEvent(j,{type:'started',id:j.id},1100);
  const stats={outputTokens:319,promptTokens:680,generationMs:7696.14,tps:41.32,elapsedMs:9490,firstTokenMs:1794};
  j = applyGenerationEvent(j,{type:'metrics',id:j.id,stats},10590);
  j = applyGenerationEvent(j,{type:'delta',id:j.id,text:'<html><body>中文</body></html>'},10590);
  j = applyGenerationEvent(j,{type:'done',id:j.id,html:j.raw,incomplete:false,seconds:9.5},10600);
  assert.equal(j.endedAt,10600); assert.deepEqual(j.stats,stats);
  const a=summarizePerformance([j],11000), b=summarizePerformance([j],90000);
  assert.deepEqual(a,b); assert.equal(a.elapsedMs,9600); assert.equal(a.inferenceMs,9500);
});
