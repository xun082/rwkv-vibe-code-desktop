import { test } from 'node:test';
import assert from 'node:assert/strict';
import { previewDocument, extractPreview, finalPreviewDocument } from '../src/preview.ts';

test('the restrictive CSP precedes all untrusted document content', () => {
  const malicious = '<html><head><meta http-equiv="Content-Security-Policy" content="default-src *"></head><body><script>parent.document.body.innerHTML="bad"</script></body></html>';
  const doc = previewDocument(malicious);
  assert.ok(doc.indexOf("default-src 'none'") < doc.indexOf(malicious));
  assert.ok(doc.includes("connect-src 'none'"));
  assert.ok(doc.includes("form-action 'none'"));
});

test('completed previews replace Tailwind CDN with the bundled compiler and permit reference photos', () => {
  const doc = finalPreviewDocument('<html><head><script src="https://cdn.tailwindcss.com"></script></head><body><img src="https://images.unsplash.com/photo-1"></body></html>', 'test', 'function(){}', undefined, 'window.compilerLoaded=true;');
  assert.ok(doc.includes('<script>window.compilerLoaded=true;</script>'));
  assert.ok(!doc.includes('src="https://cdn.tailwindcss.com"'));
  assert.ok(doc.includes('img-src data: blob: https://images.unsplash.com'));
  assert.ok(doc.includes("connect-src 'none'"));
});

test('extracts mixed-case HTML from explanations without inventing a result', () => {
  assert.equal(extractPreview('Here it is: ```html\n<!DOCTYPE html><html>你好</html>\n```'), '<!DOCTYPE html><html>你好</html>');
  assert.equal(extractPreview('No HTML was generated'), '');
  assert.equal(extractPreview('```html\n<!DOCTYPE html><html>你好</html>\n```\n设计说明'), '<!DOCTYPE html><html>你好</html>');
});
