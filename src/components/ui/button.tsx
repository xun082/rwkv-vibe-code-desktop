// Adapted from the shadcn/ui New York Button. See LICENSE in this directory.
import { Slot } from '@radix-ui/react-slot';
import type { ComponentProps } from 'react';

type ButtonProps = ComponentProps<'button'> & {
  variant?: 'default' | 'outline';
  size?: 'default' | 'icon' | 'icon-sm';
  asChild?: boolean;
};

export function Button({ variant = 'default', size = 'default', asChild = false, ...props }: ButtonProps) {
  const Component = asChild ? Slot : 'button';
  return <Component data-slot="button" data-variant={variant} data-size={size} {...props}/>;
}
