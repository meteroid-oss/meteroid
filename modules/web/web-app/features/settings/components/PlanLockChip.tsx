import { cn } from '@md/ui'
import { LockIcon } from 'lucide-react'

// Inline marker of a capability this organization does not have. Sits next to a disabled
// control or a section title.
export const PlanLockChip = ({
  label = 'Enterprise',
  noLabel = false,
  className,
}: {
  label?: string
  noLabel?: boolean
  className?: string
}) => (
  <span
    className={cn(
      'inline-flex items-center gap-1 rounded-full border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-[10px] font-semibold text-warning',
      className
    )}
  >
    <LockIcon size={10} />
    {!noLabel && label}
  </span>
)
