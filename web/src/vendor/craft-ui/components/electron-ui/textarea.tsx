// @ts-nocheck
/**
 * Vendored from craft-agents-oss v0.8.12 (Apache-2.0).
 * Source: https://github.com/lukilabs/craft-agents-oss/blob/d9c585b8a1e5dc4557e3006b0fffaaa587f5dbb7/apps/electron/src/renderer/components/ui/textarea.tsx
 */
import * as React from "react"

import { cn } from "~vendor/lib/utils"

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "border-foreground/15 placeholder:text-muted-foreground focus-visible:border-foreground/30 focus-visible:ring-foreground/15 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:bg-foreground/5 flex field-sizing-content min-h-16 w-full rounded-md border bg-transparent px-3 py-2 text-base shadow-xs transition-[color,box-shadow] outline-none focus-visible:ring-1 disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        className
      )}
      {...props}
    />
  )
}

export { Textarea }
