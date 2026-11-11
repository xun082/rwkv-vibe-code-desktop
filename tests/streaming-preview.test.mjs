import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { JSDOM, VirtualConsole } from 'jsdom';
import { streamingDocument } from '../src/preview.ts';
import { installStreamRuntime } from '../src/preview-runtime.ts';

const require = createRequire(import.meta.url);
const source = readFileSync(require.resolve('morphdom/dist/morphdom-umd.js'), 'utf8');
function canvas(channel = 'test-channel', tailwind = '') {
  const messages = [];
  const observers = [];
  const virtualConsole = new VirtualConsole();
  virtualConsole.on('jsdomError', error => { if (error.type !== 'css-parsing') throw error; });
  const dom = new JSDOM(streamingDocument(source, installStreamRuntime.toString(), channel, '', tailwind), { runScripts: 'dangerously', pretendToBeVisual: true, virtualConsole, beforeParse(window) {
    const NativeObserver = window.MutationObserver;
    window.MutationObserver = class extends NativeObserver { constructor(callback) { super(callback); observers.push(this); } };
  } });
  dom.window.postMessage = data => messages.push(data);
  let sequence = 0;
  const render = (html, overrideChannel = channel, overrideSource = dom.window) => {
    dom.window.dispatchEvent(new dom.window.MessageEvent('message', { source: overrideSource, data: { type: 'render-html', channel: overrideChannel, html, sequence: ++sequence } }));
  };
  return { dom, window: dom.window, document: dom.window.document, messages, render, disconnect: () => observers.forEach(observer => observer.disconnect()) };
}

test('unfinished streamed HTML becomes visible before a closing document arrives', () => {
  const c = canvas();
  c.render('<!DOCTYPE html><html><body><h1 id="hero">正在生');
  assert.equal(c.document.querySelector('#hero')?.textContent, '正在生');
  const sameHeading = c.document.querySelector('#hero');
  c.render('<!DOCTYPE html><html><body><h1 id="hero">正在生长的网页</h1><section id="cards"><p>卡片一');
  assert.equal(c.document.querySelector('#hero'), sameHeading, 'the canvas must patch the existing DOM rather than replace the iframe');
  assert.equal(sameHeading.textContent, '正在生长的网页');
  assert.equal(c.document.querySelector('#cards')?.textContent, '卡片一');
  assert.ok(c.messages.some(message => message.type === 'preview-rendered' && message.hasContent && message.updates === 2));
  c.window.close();
});

test('bundled Tailwind compiles streamed classes before completion and survives later head patches', async () => {
  const tailwind = readFileSync(new URL('../src/vendor/tailwind-play-3.4.17.js', import.meta.url), 'utf8');
  const c = canvas('tailwind-preview', tailwind);
  const prefix = '<html><head><script src="https://cdn.tailwindcss.com"></script></head><body>';
  const waitFor = async check => {
    const end = Date.now() + 5000;
    while (!check() && Date.now() < end) await new Promise(resolve => setTimeout(resolve, 10));
    assert.ok(check());
  };
  try {
    c.render(prefix + '<h1 class="text-3xl font-bold">咖啡馆');
    await waitFor(() => c.document.querySelector('#__preview_tailwind')?.textContent.includes('.text-3xl'));
    const sheet = c.document.querySelector('#__preview_tailwind');
    c.render(prefix + '<h1 class="text-3xl font-bold">咖啡馆</h1><section class="grid grid-cols-3">菜单');
    await waitFor(() => sheet.textContent.includes('.grid-cols-3'));
    assert.equal(c.document.querySelector('#__preview_tailwind'), sheet);
    assert.equal(c.document.querySelectorAll('#__preview_tailwind').length, 1);
    assert.equal(c.document.querySelector('h1').textContent, '咖啡馆');
    assert.equal(c.document.querySelector('script[src]'), null, 'no network script is required');
    // Let the compiler's mutation-triggered promises settle before destroying its window.
    await new Promise(resolve => setTimeout(resolve, 20));
  } finally { c.disconnect(); await new Promise(resolve => setTimeout(resolve, 0)); c.window.close(); }
});

test('streams can update CSS without erasing the fixed CSP or executing model scripts', () => {
  const c = canvas();
  const policy = c.document.querySelector('meta[http-equiv]');
  c.render('<html><head><meta http-equiv="Content-Security-Policy" content="default-src *"><style>h1{color:red}</style></head><body onload="window.executed=true"><h1 onclick="window.executed=true">Hello</h1><script>window.executed=true</script><img src=x onerror="window.executed=true"><template><script>bad()</script></template>');
  assert.equal(c.window.executed, undefined);
  assert.equal(c.document.querySelector('script'), null);
  assert.equal(c.document.querySelector('[onload],[onclick],[onerror]'), null);
  assert.equal(c.document.querySelector('template').content.querySelector('script'), null);
  assert.equal(c.document.querySelector('meta[http-equiv]'), policy);
  assert.ok(policy.content.includes("default-src 'none'"));
  assert.equal(c.document.querySelectorAll('meta[http-equiv]').length, 1);
  assert.equal(c.document.querySelector('style').textContent, 'h1{color:red}');
  c.render('<html><head><style>h1{color:blue}</style></head><body><h1>Hello again</h1>');
  assert.equal(c.document.querySelector('style').textContent, 'h1{color:blue}');
  assert.equal(c.document.querySelector('meta[http-equiv]'), policy);
  c.window.close();
});

test('two concurrent canvases remain isolated, including late messages to the wrong channel', () => {
  const a = canvas('channel-a'), b = canvas('channel-b');
  a.render('<html><body><h1>A 的网页');
  b.render('<html><body><h1>B 的网页');
  a.render('<html><body><h1>不应该出现', 'channel-b');
  b.render('<html><body><h1>错误来源', 'channel-b', null);
  assert.equal(a.document.body.textContent, 'A 的网页');
  assert.equal(b.document.body.textContent, 'B 的网页');
  a.window.close(); b.window.close();
});

test('CSS source alone is not counted as a visible first frame', () => {
  const c = canvas();
  c.render('<html><body><style>body{background:red}</style>');
  assert.equal(c.messages.find(message => message.type === 'preview-rendered')?.hasContent, false);
  c.window.close();
});

test('the document prefill stays blank until actual model-authored content arrives', () => {
  const c = canvas();
  const rust = readFileSync(new URL('../src-tauri/src/prompt.rs', import.meta.url), 'utf8');
  const prefill = rust.match(/HTML_PREFILL: &str = r#"(.*?)"#;/s)[1];
  c.render(prefill);
  assert.equal(c.messages.some(message => message.type === 'preview-content-ready'), false);
  c.render(prefill + ' class="bg-amber-50"><header><nav><a>慢半拍');
  assert.equal(c.messages.filter(message => message.type === 'preview-content-ready').length, 1);
  assert.equal(c.document.body.textContent, '慢半拍');
  assert.equal(c.document.body.className, 'bg-amber-50');
  c.window.close();
});

test('first nonempty preview is acknowledged once without relying on iframe animation frames', () => {
  const c = canvas();
  c.window.requestAnimationFrame = () => { throw new Error('WKWebView may pause rAF'); };
  c.render('<html><body><style>body{background:red}</style>');
  assert.equal(c.messages.filter(message => message.type === 'preview-content-ready').length, 0);
  c.render('<html><body><h1>第一屏</h1>');
  assert.equal(c.messages.filter(message => message.type === 'preview-content-ready').length, 1);
  c.render('<html><body><h1>第一屏</h1><p>后续内容</p>');
  assert.equal(c.messages.filter(message => message.type === 'preview-content-ready').length, 1);
  c.window.close();
});
