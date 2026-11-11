import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('the local inference prompt matches the reference webpage prefix verbatim', () => {
  const original = readFileSync(new URL('./fixtures/reference-prompt.txt', import.meta.url), 'utf8').trimEnd();
  const rust = readFileSync(new URL('../src-tauri/src/prompt.rs', import.meta.url), 'utf8');
  const prefix = rust.match(/WEBPAGE_PREFIX: &str = r#"(.*?)"#;/s)?.[1];
  assert.equal(prefix, original);
  assert.ok(!rust.includes('EXAMPLE'), 'short few-shot templates must not bias the requested design');
  const backend = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  assert.ok(!backend.includes('"grammar"'), 'the output must not be forced into a predefined page structure');
});
