import { cn } from '@md/ui'

import type { ReactNode } from 'react'

/** One of the compact boxes under a wallet's balance: a label, a figure and a line or two. */
export const WalletBox = ({
  label,
  value,
  valueClassName,
  children,
}: {
  label: ReactNode
  value: ReactNode
  valueClassName?: string
  children?: ReactNode
}) => (
  <div className="flex min-w-0 flex-col gap-1 rounded-lg border border-border p-3">
    <div className="text-xs text-muted-foreground">{label}</div>
    <div className={cn('text-sm font-semibold tabular-nums text-foreground', valueClassName)}>
      {value}
    </div>
    {children && <div className="space-y-0.5 text-xs text-muted-foreground">{children}</div>}
  </div>
)
