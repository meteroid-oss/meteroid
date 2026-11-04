import { cn } from '@md/ui'
import { formatDistanceToNowStrict } from 'date-fns'

import { compactQuantity } from '@/lib/utils/format'

/**
 * Usage against a limit for the current billing cycle, with the elapsed share of the cycle as a
 * labelled tick: ahead of the elapsed tick = burning fast.
 */
export const UsageMeter = ({
  featureName,
  consumed,
  limit,
  pct,
  elapsedPct,
  measuredAt,
  overageAllowed,
  onClick,
  className,
}: {
  featureName: string
  consumed: string | number
  limit: string | number
  /** 0–100+, as computed by the server; derived from consumed / limit when absent. */
  pct?: number
  elapsedPct?: number
  /** When the snapshot was taken: detectors revisit large tenants over several hours. */
  measuredAt?: string
  /** A soft limit: usage past it is billed as overage, shown as "Over limit", not as a breach. */
  overageAllowed?: boolean
  onClick?: () => void
  className?: string
}) => {
  const limitValue = Number(limit)
  const value = pct ?? (limitValue > 0 ? (Number(consumed) / limitValue) * 100 : 0)
  const over = value >= 100 && !overageAllowed
  const softOver = value >= 100 && !!overageAllowed
  const approaching = value >= 80 && value < 100
  const elapsed =
    elapsedPct !== undefined ? Math.round(Math.min(100, Math.max(0, elapsedPct))) : undefined
  const age = measuredAt ? `${formatDistanceToNowStrict(new Date(measuredAt))} ago` : undefined
  const sentence = `${compactQuantity(consumed)} of ${compactQuantity(limit)} ${featureName} used this cycle (${Math.round(value)}%)${elapsed !== undefined ? `, period ${elapsed}% elapsed` : ''}${age ? `, measured ${age}` : ''}`

  const bar = (
    <span className="flex min-w-0 flex-col gap-1">
      <span className="flex items-baseline justify-between gap-2 text-xs">
        <span className="truncate text-muted-foreground">{featureName}</span>
        <span
          className={cn(
            'tabular-nums',
            over ? 'font-medium text-destructive' : approaching ? 'text-warning' : 'text-foreground'
          )}
        >
          {Math.round(value)}%
        </span>
      </span>
      <span
        role="progressbar"
        aria-label={sentence}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.min(100, Math.round(value))}
        className="relative block h-1.5 w-full overflow-hidden rounded-full bg-muted"
      >
        <span
          className={cn(
            'absolute inset-y-0 left-0 rounded-full',
            over ? 'bg-destructive' : approaching ? 'bg-warning' : 'bg-brand'
          )}
          style={{ width: `${Math.min(100, Math.max(0, value))}%` }}
        />
        {elapsed !== undefined && (
          <span
            aria-hidden
            className="absolute -inset-y-px w-0.5 bg-foreground/60"
            style={{ left: `${elapsed}%` }}
          />
        )}
      </span>
      <span className="flex flex-wrap items-center gap-x-1 text-xs tabular-nums text-muted-foreground">
        <span>
          {compactQuantity(consumed)} / {compactQuantity(limit)}
          {softOver && ' · Over limit'}
          {age && ` · ${age}`}
        </span>
        {elapsed !== undefined && (
          <span className="inline-flex items-center gap-1">
            ·
            <span aria-hidden className="inline-block h-2.5 w-0.5 bg-foreground/60" />
            period {elapsed}% elapsed
          </span>
        )}
      </span>
    </span>
  )

  return onClick ? (
    <button
      type="button"
      onClick={onClick}
      title={sentence}
      className={cn('block w-full rounded text-left hover:bg-muted/40', className)}
    >
      {bar}
    </button>
  ) : (
    <span className={cn('block w-full', className)} title={sentence}>
      {bar}
    </span>
  )
}
