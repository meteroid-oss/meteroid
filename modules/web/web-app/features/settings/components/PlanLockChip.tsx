import { cn } from '@md/ui'
import { LockIcon } from 'lucide-react'

// Inline "this is a paid capability" marker. Sits next to a disabled control or
// a section title; the shared vocabulary for partial paywalls across settings.
export const PlanLockChip = ({
  label = 'Paid',
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
