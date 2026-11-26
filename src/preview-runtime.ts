// Serialized into the sandbox iframe. Keep this function self-contained.
export function installStreamRuntime(morph: (from: Node, to: Node, options: object) => Node, channel: string, tailwindSource = '') {
  const policy = document.querySelector('meta[http-equiv="Content-Security-Policy"]')!;
  let updates = 0;
  let painted = false;
  let hasContent = false;
  const notify = (data: object) => parent.postMessage({ channel, ...data }, '*');
  let tailwindLoaded = false;

  const motion = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  const maxTop = () => {
    const scroller = document.scrollingElement || document.documentElement;
    return Math.max(0, scroller.scrollHeight - scroller.clientHeight);
  };
  let expectedTop = scrollY;
  let timer: number | null = null;
  let startedScroll = false;
  let manualScroll = false;
  const canScroll = () => hasContent && !manualScroll && !motion?.matches && !document.hidden;
  const pauseScroll = () => {
    manualScroll = true;
    if (timer !== null) window.clearTimeout(timer);
    timer = null;
  };
  const detectManualPosition = () => {
    if (Math.abs(scrollY - Math.min(expectedTop, maxTop())) > 1) pauseScroll();
  };
  const moveTo = (x: number, y: number) => {
    window.scrollTo({ left: x, top: y, behavior: 'instant' });
    expectedTop = scrollY;
  };
  const scheduleScroll = () => {
    if (timer !== null || !canScroll() || maxTop() <= scrollY) return;
    // Give the first screen time to read, then move without catching up after a stalled frame.
    timer = window.setTimeout(() => {
      timer = null;
      detectManualPosition();
      if (!canScroll()) return;
      const from = scrollY;
      const to = Math.min(maxTop(), from + 2);
      if (to <= from) return;
      moveTo(scrollX, to);
      if (scrollY > from) {
        startedScroll = true;
        scheduleScroll();
      }
    }, startedScroll ? 40 : 1000);
  };
  window.addEventListener('scroll', detectManualPosition, { passive: true });
  window.addEventListener('wheel', pauseScroll, { passive: true, capture: true });
  window.addEventListener('touchmove', pauseScroll, { passive: true, capture: true });
  window.addEventListener('keydown', event => {
    if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(event.key)) pauseScroll();
  }, true);
  window.addEventListener('resize', scheduleScroll);
  window.addEventListener('load', scheduleScroll, true);
  document.addEventListener('visibilitychange', scheduleScroll);
  motion?.addEventListener?.('change', scheduleScroll);
  if (window.ResizeObserver) {
    const observer = new ResizeObserver(scheduleScroll);
    observer.observe(document.documentElement);
    observer.observe(document.body);
  }

  // Preserve the compiler's stylesheet when morphing model-authored head nodes.
  new MutationObserver(records => {
    for (const record of records) for (const node of record.addedNodes) {
      if (node instanceof HTMLStyleElement && !node.hasAttribute('data-preview-model') && !node.hasAttribute('data-preview-runtime')) {
        node.id = '__preview_tailwind'; node.setAttribute('data-preview-runtime', '');
      }
    }
  }).observe(document.head, { childList: true });
  const options = { onBeforeElUpdated: (from: Element, to: Element) => from === policy || from.hasAttribute('data-preview-runtime') || from.isEqualNode(to) || (from === document.activeElement && ['INPUT', 'TEXTAREA', 'SELECT'].includes(from.tagName)) ? false : undefined };
  window.addEventListener('message', event => {
    if (event.source !== parent || event.data?.channel !== channel) return;
    if (event.data.type === 'preview-wheel' && event.data.deltaY && Number.isFinite(event.data.deltaY)) pauseScroll();
    if (event.data.type !== 'render-html') return;
    // The native worker caps each lane at 8 MiB in UTF-8. Valid output can use up
    // to the same number of UTF-16 code units in this iframe.
    if (typeof event.data.html !== 'string' || event.data.html.length > 8 * 1024 * 1024) {
      notify({ type: 'preview-error', sequence: event.data.sequence, message: 'Invalid preview snapshot' });
      return;
    }
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
      detectManualPosition();
      morph(document.head, nextHead, options);
      morph(document.body, parsed.body, options);
      document.documentElement.lang = parsed.documentElement.lang;
      // Styles can target html itself, rather than body.
      document.documentElement.className = parsed.documentElement.className;
      document.documentElement.style.cssText = parsed.documentElement.style.cssText;
      if (scrollX !== x || scrollY !== y) moveTo(x, y);
      expectedTop = scrollY;
      updates++;
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
        acceptNode: node => node.textContent?.trim() && !node.parentElement?.closest('style,script,noscript,template') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
      });
      hasContent = !!walker.nextNode() || !!document.body.querySelector('img,svg,canvas,video,input,button');
      notify({ type: 'preview-rendered', sequence: event.data.sequence, updates, hasContent, x: scrollX, y: scrollY });
      scheduleScroll();
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

// Shared by the evolving canvas and the final document.
export function installPreviewControls(channel: string) {
  // Keep native scrolling input, such as touch and keys, inside the iframe.
  const boundary = document.createElement('style');
  boundary.setAttribute('data-preview-runtime', '');
  boundary.textContent = ':root, body { overscroll-behavior-y: contain !important }';
  document.head.append(boundary);

  const scrollTarget = (element: Element | null) => {
    const root = document.scrollingElement || document.documentElement;
    const rootOverflow = getComputedStyle(root).overflowY;
    const body = document.body;
    // When html leaves overflow visible, body supplies the viewport's overflow.
    const bodyIsViewport = root === document.documentElement && rootOverflow === 'visible';
    for (let region = element; region && region !== root; region = region.parentElement) {
      if (region === body && bodyIsViewport) break;
      if (region.scrollHeight > region.clientHeight && ['auto', 'scroll', 'overlay'].includes(getComputedStyle(region).overflowY)) return region;
    }
    const overflow = bodyIsViewport && body ? getComputedStyle(body).overflowY : rootOverflow;
    return overflow === 'hidden' || overflow === 'clip' ? null : root;
  };

  let x = scrollX, y = scrollY;
  const reportPosition = () => {
    if (x === scrollX && y === scrollY) return;
    x = scrollX; y = scrollY;
    parent.postMessage({ type: 'preview-position', channel, x, y }, '*');
  };
  const scrollAtPoint = (x: number, y: number, deltaY: number, deltaMode: number) => {
    const scroller = scrollTarget(document.elementFromPoint(x, y));
    if (!scroller) return;
    const delta = deltaY * (deltaMode === 2 ? scroller.clientHeight : deltaMode === 1 ? 16 : 1);
    if (scroller === (document.scrollingElement || document.documentElement)) {
      window.scrollTo({ left: scrollX, top: scrollY + delta, behavior: 'instant' });
      reportPosition();
    } else {
      scroller.scrollTo({ top: scroller.scrollTop + delta, behavior: 'instant' });
    }
  };
  window.addEventListener('wheel', event => {
    const deltaMode = event.deltaMode;
    if (!event.cancelable || !event.deltaY || event.ctrlKey) return;
    event.preventDefault();
    scrollAtPoint(event.clientX, event.clientY, event.deltaY, deltaMode);
  }, { passive: false, capture: true });

  const reportPaint = () => {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode: node => node.textContent?.trim() && !node.parentElement?.closest('style,script,noscript,template') ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
    });
    if (walker.nextNode() || document.body.querySelector('img,svg,canvas,video,input,button')) {
      parent.postMessage({ type: 'preview-content-ready', channel }, '*');
    }
  };
  window.addEventListener('load', reportPaint);
  window.addEventListener('scroll', reportPosition, { passive: true });
  window.addEventListener('message', event => {
    const wheel = event.data;
    if (event.source !== parent || wheel?.channel !== channel || wheel.type !== 'preview-wheel') return;
    if (!Number.isFinite(wheel.x) || !Number.isFinite(wheel.y) || !Number.isFinite(wheel.deltaY) || !wheel.deltaY) return;

    scrollAtPoint(wheel.x, wheel.y, wheel.deltaY, wheel.deltaMode);
  });
  document.currentScript?.remove();
}
