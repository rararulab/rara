// Adapted from vercel/ai-chatbot (Apache-2.0)
// https://github.com/vercel/ai-chatbot/blob/main/components/ui/spinner.tsx
import { Loader2Icon } from 'lucide-react';

import { cn } from '@/lib/utils';

function Spinner({ className, ...props }: React.ComponentProps<'svg'>) {
  return (
    <Loader2Icon
      role="status"
      aria-label="Loading"
      className={cn('size-4 animate-spin', className)}
      {...props}
    />
  );
}

export { Spinner };
