// Serialized into the sandbox iframe. Keep this function self-contained.
export function installStreamRuntime(morph: (from: Node, to: Node, options: object) => Node, channel: string, tailwindSource = '') {
  const policy = document.querySelector('meta[http-equiv="Content-Security-Policy"]')!;
  let updates = 0;
  let painted = false;
  const notify = (data: object) => parent.postMessage({ channel, ...data }, '*');
  let tailwindLoaded = false;
  // Preserve the compiler's stylesheet when morphing model-authored head nodes.
  new MutationObserver(records => {
    for (const record of records) for (const node of record.addedNodes) {
      if (node instanceof HTMLStyleElement && !node.hasAttribute('data-preview-model')) {
        node.id = '__preview_tailwind'; node.setAttribute('data-preview-runtime', '');
      }
    }
  }).observe(document.head, { childList: true });
  const options = { onBeforeElUpdated: (from: Element, to: Element) => from === policy || from.hasAttribute('data-preview-runtime') || from.isEqualNode(to) || (from === document.activeElement && ['INPUT', 'TEXTAREA', 'SELECT'].includes(from.tagName)) ? false : undefined };
  window.addEventListener('message', event => {
    if (event.source !== parent || event.data?.channel !== channel || event.data?.type !== 'render-html' || typeof event.data.html !== 'string') return;
    if (event.data.html.length > 2 * 1024 * 1024) return;
    try {
      // DOMParser repairs unfinished HTML without executing the model's scripts.
      const parsed = new DOMParser().parseFromString(event.data.html, 'text/html');
      const usesTailwind = [...parsed.querySelectorAll('script[src],link[href]')].some(node => /(?:cdn\.tailwindcss\.com|(?:@tailwindcss\/browser|tailwindcss)[@/])/i.test(node.getAttribute('src') || node.getAttribute('href') || ''));
      if (usesTailwind && tailwindSource && !tailwindLoaded) {
        tailwindLoaded = true;
        const compiler = document.createElement('script');
        compiler.textContent = tailwindSource; document.head.append(compiler); compiler.remove();
      }
      const sanitize = (root: Document | DocumentFragment) => {
        root.querySelectorAll('script,iframe,frame,object,embed,base,link,meta[http-equiv]').forEach(node => node.remove());
        root.querySelectorAll('*').forEach(element => {
          for (const attribute of Array.from(element.attributes)) {
            if (/^on/i.test(attribute.name) || attribute.name === 'autofocus' || (['href', 'src', 'action', 'formaction', 'xlink:href'].includes(attribute.name) && /^\s*javascript:/i.test(attribute.value))) element.removeAttribute(attribute.name);
          }
          if (element instanceof HTMLTemplateElement) sanitize(element.content);
        });
      };
      sanitize(parsed);
      const nextHead = document.createElement('head');
      nextHead.append(policy.cloneNode(true));
      parsed.querySelectorAll('style').forEach(node => { node.setAttribute('data-preview-model', ''); node.removeAttribute('data-preview-runtime'); if (node.id === '__preview_tailwind') node.removeAttribute('id'); });
      document.head.querySelectorAll('[data-preview-runtime]').forEach(node => nextHead.append(node.cloneNode(true)));
      parsed.head.querySelectorAll('style,title,meta[charset],meta[name="viewport"]').forEach(node => nextHead.append(node.cloneNode(true)));
      const x = scrollX, y = scrollY;
      morph(document.head, nextHead, options);
      morph(document.body, parsed.body, options);
      document.documentElement.lang = parsed.documentElement.lang;
      // Styles can target html itself, rather than body.
      document.documentElement.className = parsed.documentElement.className;
      document.documentElement.style.cssText = parsed.documentElement.style.cssText;
      if (x || y) scrollTo(x, y);
      updates++;
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
        acceptNode: node => node.textContent?.trim() && !node.parentElement?.closest('style,script,noscript,template') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
      });
      const hasContent = !!walker.nextNode() || !!document.body.querySelector('img,svg,canvas,video,input,button');
      notify({ type: 'preview-rendered', sequence: event.data.sequence, updates, hasContent, x: scrollX, y: scrollY });
      if (hasContent && !painted) {
        painted = true;
        // Report a non-empty preview DOM synchronously. WKWebView may suspend
        // iframe animation callbacks even while the document is being updated.
        notify({ type: 'preview-content-ready' });
      }
    } catch (error) {
      notify({ type: 'preview-error', sequence: event.data.sequence, message: String(error) });
    }
  });
  document.currentScript?.remove();
  notify({ type: 'preview-ready' });
}

// Height reporting and wheel relay adapted from rwkv-vibe-code/src/lib/preview.ts.
export function installPreviewControls(channel: string) {
  let last = 0;
  const post = () => {
    const height = Math.max(document.documentElement.scrollHeight, document.body?.scrollHeight || 0);
    if (!height || Math.abs(height - last) < 8) return;
    last = height;
    parent.postMessage({ type: 'preview-height', channel, height }, '*');
  };
  const reportPaint = () => {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode: node => node.textContent?.trim() && !node.parentElement?.closest('style,script,noscript,template') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
    });
    if (walker.nextNode() || document.body.querySelector('img,svg,canvas,video,input,button')) {
      parent.postMessage({ type: 'preview-content-ready', channel }, '*');
    }
  };
  post(); window.addEventListener('load', post); window.addEventListener('load', reportPaint);
  if (window.ResizeObserver) new ResizeObserver(post).observe(document.documentElement);
  window.addEventListener('message', event => {
    if (event.source !== parent || event.data?.channel !== channel || event.data.type !== 'preview-scroll' || !Number.isFinite(event.data.top)) return;
    window.scrollTo(0, event.data.top);
  });
  document.currentScript?.remove();
}
