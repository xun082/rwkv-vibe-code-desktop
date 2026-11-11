import { autoCompleteHTML } from './lib/html.ts';
// This CSP is inserted before ALL model-authored content. The iframe gets an opaque origin.
// The reference prompt uses Unsplash photos. Inference stays local; photos may load online.
export function previewDocument(html: string, position = { x: 0, y: 0 }) {
  const policy = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob: https://images.unsplash.com https://plus.unsplash.com https://source.unsplash.com; font-src data:; connect-src 'none'; media-src data: blob:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  const restore = position.x || position.y ? `<script>requestAnimationFrame(function(){scrollTo(${Number(position.x) || 0},${Number(position.y) || 0})})</script>` : '';
  return `<!DOCTYPE html><html><head><meta http-equiv="Content-Security-Policy" content="${policy}"><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"></head><body>${html}${restore}</body></html>`;
}
export function extractPreview(raw: string) {
  const match = /<!doctype\s+html|<html[\s>]/i.exec(raw);
  if (!match) return '';
  const html = raw.slice(match.index).replace(/```\s*$/, '').trimEnd();
  const end = /<\/html\s*>/i.exec(html);
  return autoCompleteHTML(end ? html.slice(0, end.index + end[0].length) : html);
}

// The runtime is trusted app code, not model output. Its CSP remains before all updates.
export function streamingDocument(morphdom: string, runtime: string, channel: string, controls = '', tailwind = '') {
  const script = `${morphdom}\n;(${runtime})(window.morphdom, ${JSON.stringify(channel)}, ${JSON.stringify(tailwind)});${controls ? `(${controls})(${JSON.stringify(channel)});` : ''}`.replace(/<\/script/gi, '<\\/script');
  return previewDocument(`<script>${script}</script>`);
}

export function finalPreviewDocument(html: string, channel: string, controls: string, position = { x: 0, y: 0 }, tailwind = '') {
  // Use the pinned bundled compiler in place of the reference's CDN script.
  // Other model-authored scripts remain confined to the opaque-origin iframe.
  if (tailwind) html = html.replace(/<script\b[^>]*\bsrc\s*=\s*["']https?:\/\/(?:cdn\.tailwindcss\.com[^"']*|(?:cdn\.jsdelivr\.net|unpkg\.com)\/[^"']*(?:@tailwindcss\/browser|tailwindcss)[^"']*)["'][^>]*>[\s\S]*?<\/script\s*>/gi, () => `<script>${tailwind.replace(/<\/script/gi, '<\\/script')}</script>`);
  const script = `(${controls})(${JSON.stringify(channel)});`.replace(/<\/script/gi, '<\\/script');
  return previewDocument(`${html}<script>${script}</script>`, position);
}
