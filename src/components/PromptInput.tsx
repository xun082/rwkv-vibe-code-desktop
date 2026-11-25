import { useLayoutEffect, useRef, type RefObject } from 'react';
import { ArrowUp } from 'lucide-react';
import { CONCURRENCY_OPTIONS, DEFAULT_CONCURRENCY, MAX_CONCURRENCY, type ConcurrencyMode, type HardwareProfile } from '../types';
import { MAX_PROMPT_LENGTH, splitPromptInput } from '../lib/batch-prompts';
import { Button } from './ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';

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
  const canSubmit = !disabled && !!value.trim();

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
      placeholder={compact ? '描述网页需求，多条需求用单独一行 --- 分隔...' : '描述你的想法...'}
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
        <SelectTrigger size={compact ? 'sm' : 'default'} aria-label="并发数量" title={parallelismHint}>
          <SelectValue/>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="auto">自动 · {recommended} 路</SelectItem>
          {CONCURRENCY_OPTIONS.map(lanes => <SelectItem key={lanes} value={String(lanes)}>{lanes} 路</SelectItem>)}
        </SelectContent>
      </Select>
      <Button type="submit" size={compact ? 'icon-sm' : 'icon'} disabled={!canSubmit}
        aria-label="生成网页" title="生成网页"><ArrowUp size={compact ? 16 : 18} aria-hidden="true"/></Button>
    </div>
  </form>;
}
