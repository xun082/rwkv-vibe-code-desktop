import { useLayoutEffect, useMemo, useRef, useState } from 'react';
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
  const position = useRef({ x: 0, y: 0 });
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
      if (event.data.type === 'preview-error') queue.current.reject(event.data.sequence);
    };
    window.addEventListener('message', listener);
    return () => window.removeEventListener('message', listener);
  }, [channel]);
  useLayoutEffect(() => { queue.current.reset(); }, [streaming]);
  // Push every committed delta directly. The ready/onLoad handshake also pushes
  // the latest snapshot, so neither an interval nor a closing HTML tag is needed.
  useLayoutEffect(() => { if (streaming) push.current(); }, [html, streaming]);
  useLayoutEffect(() => {
    const element = surface.current;
    if (!element || interactive) return;
    // The card keeps clicks for opening detail. Relay its wheel to the iframe,
    // which can hit-test the generated page and own the gesture at its edges.
    const wheel = (event: WheelEvent) => {
      const deltaMode = event.deltaMode;
      if (!event.deltaY || event.ctrlKey) return;
      event.preventDefault();
      event.stopPropagation();
      const iframe = frame.current;
      if (!iframe || scale <= 0) return;
      const bounds = iframe.getBoundingClientRect();
      iframe.contentWindow?.postMessage({
        type: 'preview-wheel', channel,
        x: (event.clientX - bounds.left) / scale,
        y: (event.clientY - bounds.top) / scale,
        deltaY: deltaMode === WheelEvent.DOM_DELTA_PIXEL ? event.deltaY / scale : event.deltaY,
        deltaMode,
      }, '*');
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [interactive, scale, channel]);
  return <div ref={surface} className="stream-surface" data-streaming={streaming}>
    <iframe ref={frame} onLoad={() => { if (streaming) { queue.current.markReady(); push.current(); } }} key={streaming ? 'live' : 'final'} title={label} sandbox="allow-scripts" referrerPolicy="no-referrer" srcDoc={streaming ? liveDoc : finalDoc} style={{ width: logicalWidth || '100%', height: logicalHeight || '100%', transform: `scale(${scale})`, pointerEvents: interactive ? 'auto' : 'none' }}/>
  </div>;
}
