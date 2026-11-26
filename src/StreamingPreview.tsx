import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import morphdomSource from 'morphdom/dist/morphdom-umd.js?raw';
import tailwindSource from './vendor/tailwind-play-3.4.17.js?raw';
import { installPreviewControls, installStreamRuntime } from './preview-runtime';
import { finalPreviewDocument, streamingDocument } from './preview';
import { PreviewQueue } from './lib/preview-queue';

interface Props { html: string; streaming?: boolean; label?: string; logicalWidth?: number; logicalHeight?: number; scale?: number; interactive?: boolean; onPainted?: () => void }

// Same isolated HTML renderer in the reference-style thumbnail and detail view.
export default function StreamingPreview({ html, streaming = false, label = '网页预览', logicalWidth, logicalHeight, scale = 1, interactive = true, onPainted }: Props) {
  const surface = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const [channel] = useState(() => crypto.randomUUID());
  const paintedCallback = useRef(onPainted); paintedCallback.current = onPainted;
  const current = useRef({ html, streaming }); current.current = { html, streaming };
  const queue = useRef(new PreviewQueue());
  const position = useRef({ x: 0, y: 0 }), contentHeight = useRef(logicalHeight || 800);
  const liveDoc = useMemo(() => streamingDocument(morphdomSource, installStreamRuntime.toString(), channel, installPreviewControls.toString(), tailwindSource), [channel]);
  // Do not rebuild a 400 KB final Tailwind document for every streaming delta.
  const finalDoc = useMemo(() => streaming ? '' : finalPreviewDocument(html, channel, installPreviewControls.toString(), position.current, tailwindSource), [html, channel, streaming]);
  const push = useRef(() => {});
  push.current = () => {
    const latest = current.current;
    if (!latest.streaming) return;
    const snapshot = queue.current.take(latest.html);
    if (snapshot) frame.current?.contentWindow?.postMessage({ type: 'render-html', channel, ...snapshot }, '*');
  };
  useLayoutEffect(() => {
    const listener = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow || event.data?.channel !== channel) return;
      if (event.data.type === 'preview-ready') { queue.current.markReady(); push.current(); }
      if (event.data.type === 'preview-rendered' || event.data.type === 'preview-position') {
        if (Number.isFinite(event.data.x) && Number.isFinite(event.data.y)) {
          position.current = { x: event.data.x, y: event.data.y };
        }
      }
      if (event.data.type === 'preview-rendered' && queue.current.acknowledge(event.data.sequence)) push.current();
      if (event.data.type === 'preview-content-ready') paintedCallback.current?.();
      if (event.data.type === 'preview-height' && Number.isFinite(event.data.height)) contentHeight.current = event.data.height;
      if (event.data.type === 'preview-error') queue.current.reject(event.data.sequence);
    };
    window.addEventListener('message', listener);
    return () => window.removeEventListener('message', listener);
  }, [channel]);
  useLayoutEffect(() => { queue.current.reset(); }, [streaming]);
  // Push every committed delta directly. The ready/onLoad handshake also pushes
  // the latest snapshot, so neither an interval nor a closing HTML tag is needed.
  useLayoutEffect(() => { if (streaming) push.current(); }, [html, streaming]);
  useEffect(() => {
    const element = surface.current;
    if (!element || interactive) return;
    // Like the reference ResultCard: route wheel input to the scaled, non-clickable
    // iframe, but let the outer grid scroll once the page reaches its ends.
    const wheel = (event: WheelEvent) => {
      const max = Math.max(0, contentHeight.current - (logicalHeight || 800));
      const y = position.current.y;
      if (max <= 0 || (event.deltaY < 0 && y <= 0) || (event.deltaY > 0 && y >= max)) return;
      event.preventDefault();
      const pixels = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? event.deltaY * 16
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? event.deltaY * element.clientHeight
        : event.deltaY;
      const top = Math.max(0, Math.min(max, y + pixels / scale));
      position.current = { ...position.current, y: top };
      frame.current?.contentWindow?.postMessage({ type: 'preview-scroll', channel, top }, '*');
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [interactive, logicalHeight, scale, channel]);
  return <div ref={surface} className="stream-surface" data-streaming={streaming}>
    <iframe ref={frame} onLoad={() => { if (streaming) { queue.current.markReady(); push.current(); } }} key={streaming ? 'live' : 'final'} title={label} sandbox="allow-scripts" referrerPolicy="no-referrer" srcDoc={streaming ? liveDoc : finalDoc} style={{ width: logicalWidth || '100%', height: logicalHeight || '100%', transform: `scale(${scale})`, pointerEvents: interactive ? 'auto' : 'none' }}/>
  </div>;
}
