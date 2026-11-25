import { useLayoutEffect, useRef, type RefObject } from 'react';
import { ArrowUp } from 'lucide-react';
import { CONCURRENCY_OPTIONS, DEFAULT_CONCURRENCY, MAX_CONCURRENCY, type ConcurrencyMode, type HardwareProfile } from '../types';
import { MAX_PROMPT_LENGTH } from '../lib/batch-prompts';
import { Button } from './ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';

interface Props {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled: boolean;
  ready: boolean;
  compact?: boolean;
  count: ConcurrencyMode;
  onCountChange: (count: ConcurrencyMode) => void;
  hardware: HardwareProfile | null;
  inputRef?: RefObject<HTMLTextAreaElement | null>;
}

export default function PromptInput({ value, onChange, onSubmit, disabled, ready, compact = false, count, onCountChange, hardware, inputRef }: Props) {
  const internalInput = useRef<HTMLTextAreaElement>(null);
  const input = inputRef || internalInput;
  const recommended = hardware?.recommendedConcurrency ?? DEFAULT_CONCURRENCY;
  const canSubmit = ready && !disabled && !!value.trim();

  useLayoutEffect(() => {
    if (!input.current) return;
    input.current.style.height = 'auto';
    input.current.style.height = `${Math.min(input.current.scrollHeight, compact ? 40 : 200)}px`;
  }, [value, compact]);

  return <form className={compact ? 'prompt-input compact' : 'prompt-input'} onSubmit={event => {
    event.preventDefault();
    if (canSubmit) onSubmit();
  }}>
    <textarea ref={input} rows={1} maxLength={MAX_CONCURRENCY * (2 * MAX_PROMPT_LENGTH + 5)}
      value={value} disabled={disabled} onChange={event => onChange(event.currentTarget.value)}
      placeholder={compact ? '描述网页需求...' : '描述你的想法...'}
      aria-label="网页生成需求" onKeyDown={event => {
        if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
          event.preventDefault();
          if (canSubmit) onSubmit();
        }
      }}/>
    <div className="prompt-actions">
      <Select value={String(count)} disabled={disabled} onValueChange={value => {
        onCountChange(value === 'auto' ? value : Number(value) as ConcurrencyMode);
      }}>
        <SelectTrigger size={compact ? 'sm' : 'default'} aria-label="并发数量">
          <SelectValue/>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="auto">自动 · {recommended} 路</SelectItem>
          {CONCURRENCY_OPTIONS.map(lanes => <SelectItem key={lanes} value={String(lanes)}>{lanes} 路</SelectItem>)}
        </SelectContent>
      </Select>
      <Button type="submit" size={compact ? 'icon-sm' : 'icon'} disabled={!canSubmit}
        aria-label="生成网页" title={ready ? '生成网页' : '本地模型尚未就绪'}><ArrowUp size={compact ? 16 : 18} aria-hidden="true"/></Button>
    </div>
  </form>;
}
