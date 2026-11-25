// Adapted from the shadcn/ui New York Select. See LICENSE in this directory.
import * as SelectPrimitive from '@radix-ui/react-select';
import { Check, ChevronDown, ChevronUp } from 'lucide-react';
import type { ComponentProps } from 'react';

export const Select = SelectPrimitive.Root;
export const SelectValue = SelectPrimitive.Value;

type TriggerProps = ComponentProps<typeof SelectPrimitive.Trigger> & { size?: 'default' | 'sm' };

export function SelectTrigger({ children, size = 'default', ...props }: TriggerProps) {
  return <SelectPrimitive.Trigger data-slot="select-trigger" data-size={size} {...props}>
    {children}
    <SelectPrimitive.Icon asChild><ChevronDown size={14} aria-hidden="true"/></SelectPrimitive.Icon>
  </SelectPrimitive.Trigger>;
}

export function SelectContent({ children, ...props }: ComponentProps<typeof SelectPrimitive.Content>) {
  return <SelectPrimitive.Portal>
    <SelectPrimitive.Content data-slot="select-content" position="popper" align="end" sideOffset={6} {...props}>
      <SelectPrimitive.ScrollUpButton data-slot="select-scroll-button"><ChevronUp size={14}/></SelectPrimitive.ScrollUpButton>
      <SelectPrimitive.Viewport data-slot="select-viewport">{children}</SelectPrimitive.Viewport>
      <SelectPrimitive.ScrollDownButton data-slot="select-scroll-button"><ChevronDown size={14}/></SelectPrimitive.ScrollDownButton>
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>;
}

export function SelectItem({ children, ...props }: ComponentProps<typeof SelectPrimitive.Item>) {
  return <SelectPrimitive.Item data-slot="select-item" {...props}>
    <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    <SelectPrimitive.ItemIndicator><Check size={14} aria-hidden="true"/></SelectPrimitive.ItemIndicator>
  </SelectPrimitive.Item>;
}
