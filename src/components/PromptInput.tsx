import { useLayoutEffect, useRef, type RefObject } from 'react';
import { ArrowUp } from 'lucide-react';
import type { Concurrency } from '../types';
import { splitPromptInput } from '../lib/batch-prompts';

// Geometry and Enter/Shift+Enter behavior from the reference ChatgptPromptInput.
interface Props { value: string; onChange: (value: string) => void; onSubmit: () => void; disabled: boolean; compact?: boolean; count: Concurrency; onCountChange: (count: Concurrency) => void; inputRef?: RefObject<HTMLTextAreaElement | null> }
export default function PromptInput({ value, onChange, onSubmit, disabled, compact = false, count, onCountChange, inputRef }: Props) {
  const internalInput = useRef<HTMLTextAreaElement>(null);
  const input = inputRef || internalInput;
  const promptCount = splitPromptInput(value).length;
  const hasMultiplePrompts = promptCount > 1;
  useLayoutEffect(() => {
    if (!input.current) return;
    input.current.style.height = 'auto';
    input.current.style.height = `${Math.min(input.current.scrollHeight, compact ? 40 : 200)}px`;
  }, [value, compact]);
  return <form className={`prompt-input ${compact ? 'compact' : ''}`} onSubmit={event => { event.preventDefault(); if (!disabled && value.trim()) onSubmit(); }}>
    <textarea ref={input} rows={1} maxLength={8020} value={value} disabled={disabled} onChange={event => onChange(event.target.value)} placeholder={compact ? '描述网页需求，多条需求用单独一行 --- 分隔...' : '描述你的想法...'} aria-label="网页生成需求" onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!disabled && value.trim()) onSubmit(); } }}/>
    <select aria-label="并发数量" title={hasMultiplePrompts ? `${promptCount} 条需求通过一个批量请求生成` : '通过一个批量请求生成的网页数量'} value={hasMultiplePrompts ? Math.min(promptCount, 4) : count} disabled={disabled || hasMultiplePrompts} onChange={event => onCountChange(Number(event.target.value) as Concurrency)}><option value={1}>1 路</option><option value={2}>2 路批量</option><option value={3}>3 路批量</option><option value={4}>4 路批量</option></select>
    <button type="submit" disabled={disabled || !value.trim()} aria-label="生成网页" title="生成网页"><ArrowUp size={compact ? 14 : 20}/></button>
  </form>;
}
