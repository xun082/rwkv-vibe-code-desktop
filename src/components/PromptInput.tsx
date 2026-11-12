import { useLayoutEffect, useRef, type RefObject } from 'react';
import { ArrowUp } from 'lucide-react';
import { CONCURRENCY_OPTIONS, DEFAULT_CONCURRENCY, MAX_CONCURRENCY, type ConcurrencyMode, type HardwareProfile } from '../types';
import { MAX_PROMPT_LENGTH, splitPromptInput } from '../lib/batch-prompts';

// Geometry and Enter/Shift+Enter behavior from the reference ChatgptPromptInput.
interface Props {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled: boolean;
  compact?: boolean;
  count: ConcurrencyMode;
  onCountChange: (count: ConcurrencyMode) => void;
  hardware: HardwareProfile | null;
  inputRef?: RefObject<HTMLTextAreaElement | null>;
}

export default function PromptInput({ value, onChange, onSubmit, disabled, compact = false, count, onCountChange, hardware, inputRef }: Props) {
  const internalInput = useRef<HTMLTextAreaElement>(null);
  const input = inputRef || internalInput;
  const promptCount = splitPromptInput(value).length;
  const recommended = hardware?.recommendedConcurrency ?? DEFAULT_CONCURRENCY;
  const recommendation = hardware?.logicalCores && hardware.memoryBytes
    ? `本机 ${hardware.logicalCores} 个逻辑处理器、${(hardware.memoryBytes / 2 ** 30).toFixed(1)} GiB 内存，建议 ${recommended} 路`
    : `自动档：${recommended} 路`;
  const requiredSlots = promptCount + promptCount % 2;
  const parallelismHint = promptCount > 1 && requiredSlots <= MAX_CONCURRENCY
    ? `${promptCount} 条需求需至少选择 ${requiredSlots} 路。${recommendation}`
    : recommendation;
  useLayoutEffect(() => {
    if (!input.current) return;
    input.current.style.height = 'auto';
    input.current.style.height = `${Math.min(input.current.scrollHeight, compact ? 40 : 200)}px`;
  }, [value, compact]);
  return <form className={`prompt-input ${compact ? 'compact' : ''}`} onSubmit={event => { event.preventDefault(); if (!disabled && value.trim()) onSubmit(); }}>
    <textarea ref={input} rows={1} maxLength={MAX_CONCURRENCY * (2 * MAX_PROMPT_LENGTH + 5)} value={value} disabled={disabled} onChange={event => onChange(event.target.value)} placeholder={compact ? '描述网页需求，多条需求用单独一行 --- 分隔...' : '描述你的想法...'} aria-label="网页生成需求" onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!disabled && value.trim()) onSubmit(); } }}/>
    <select aria-label="并发数量" title={parallelismHint} value={count} disabled={disabled} onChange={event => {
      const value = event.target.value;
      onCountChange(value === 'auto' ? value : Number(value) as ConcurrencyMode);
    }}>
      <option value="auto">自动 · {recommended} 路</option>
      {CONCURRENCY_OPTIONS.map(lanes => <option key={lanes} value={lanes}>{lanes} 路</option>)}
    </select>
    <button type="submit" disabled={disabled || !value.trim()} aria-label="生成网页" title="生成网页"><ArrowUp size={compact ? 14 : 20}/></button>
  </form>;
}
