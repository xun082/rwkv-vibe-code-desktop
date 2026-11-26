import { useCallback, useEffect, useRef, useState } from 'react';
import { appUpdateStatus, checkAppUpdate, desktop, installAppUpdate, type AppUpdateProgress } from '../bridge';

const MIN_CHECK_INTERVAL = 5 * 60_000;
const PERIODIC_CHECK_INTERVAL = 30 * 60_000;

function errorMessage(error: unknown, fallback: string) {
  if (typeof error === 'string' && error) return error;
  if (error instanceof Error && error.message) return error.message;
  return fallback;
}

export function useAppUpdate(busy: boolean) {
  const [status, setStatus] = useState<Awaited<ReturnType<typeof appUpdateStatus>> | null>(null);
  const [available, setAvailable] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [progress, setProgress] = useState<AppUpdateProgress | { phase: 'restarting' } | null>(null);
  const [feedback, setFeedback] = useState<{ kind: 'info' | 'error'; message: string } | null>(null);
  const statusRequest = useRef<ReturnType<typeof appUpdateStatus> | null>(null);
  const lastCheck = useRef<number | null>(null);
  const checkInFlight = useRef(false);
  const installInFlight = useRef(false);
  const supported = status?.supported === true;

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    statusRequest.current ??= appUpdateStatus();
    void statusRequest.current.then(value => {
      if (active) setStatus(value);
    }).catch(() => {});
    return () => { active = false; };
  }, []);

  const check = useCallback(async (manual = false) => {
    const now = Date.now();
    if (!supported || checkInFlight.current || installInFlight.current) return;
    if (!manual && lastCheck.current !== null && now - lastCheck.current < MIN_CHECK_INTERVAL) return;

    lastCheck.current = now;
    checkInFlight.current = true;
    setChecking(true);
    if (manual) setFeedback(null);
    try {
      const update = await checkAppUpdate();
      setAvailable(update?.version ?? null);
      if (manual && !update) setFeedback({ kind: 'info', message: '已是最新版本' });
    } catch (error) {
      if (manual) setFeedback({ kind: 'error', message: errorMessage(error, '无法检查更新，请重试') });
    } finally {
      checkInFlight.current = false;
      setChecking(false);
    }
  }, [supported]);

  useEffect(() => {
    if (!supported) return;
    const checkAutomatically = () => void check();
    checkAutomatically();
    const interval = window.setInterval(checkAutomatically, PERIODIC_CHECK_INTERVAL);
    window.addEventListener('focus', checkAutomatically);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener('focus', checkAutomatically);
    };
  }, [check, supported]);

  useEffect(() => {
    if (feedback?.kind !== 'info') return;
    const timeout = window.setTimeout(() => setFeedback(null), 6000);
    return () => window.clearTimeout(timeout);
  }, [feedback]);

  const install = async () => {
    if (!available || busy || installInFlight.current || checkInFlight.current) return;
    installInFlight.current = true;
    setFeedback(null);
    setProgress({ phase: 'downloading', downloadedBytes: 0, totalBytes: null });
    let receivingProgress = true;
    try {
      await installAppUpdate(available, event => {
        if (receivingProgress) setProgress(event);
      });
      setProgress({ phase: 'restarting' });
    } catch (error) {
      installInFlight.current = false;
      setProgress(null);
      setFeedback({ kind: 'error', message: errorMessage(error, '更新失败，请重试') });
    } finally {
      receivingProgress = false;
    }
  };

  return {
    supported,
    version: status?.version ?? '',
    available,
    busy,
    checking,
    installing: progress !== null,
    progress,
    feedback,
    check: () => void check(true),
    install: () => void install(),
    dismiss: () => setFeedback(null),
  };
}
