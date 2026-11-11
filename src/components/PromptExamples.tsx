import { Building2, Landmark, Coffee, Gamepad2, Timer, LayoutDashboard } from 'lucide-react';
import { EXAMPLE_PROMPTS } from '../lib/examples';

const icons = { hotel: Building2, museum: Landmark, cafe: Coffee, retroShop: Gamepad2, pomodoro: Timer, saas: LayoutDashboard };

export default function PromptExamples({ onPick }: { onPick: (prompt: string) => void }) {
  return <section className="prompt-examples" aria-label="示例需求">
    <div className="examples-heading"><span>试试这些示例</span><i/></div>
    <div className="examples-grid">{EXAMPLE_PROMPTS.map(example => {
      const Icon = icons[example.key];
      return <button key={example.key} type="button" className="example-prompt" title={example.prompt} aria-label={`使用示例：${example.label}`} onClick={() => onPick(example.prompt)}>
        <Icon size={18}/><span><strong>{example.label}</strong><small>{example.prompt}</small></span>
      </button>;
    })}</div>
  </section>;
}
