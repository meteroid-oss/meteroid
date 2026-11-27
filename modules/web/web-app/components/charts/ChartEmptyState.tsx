import { Button, cn } from '@md/ui'
import { AlertCircleIcon, RotateCwIcon } from 'lucide-react'

import type { ReactNode } from 'react'

/**
 * What a chart shows when it has nothing to plot, or failed to load. It keeps the chart's own
 * frame: a few faint grid lines and a flat dashed baseline, faded out behind the message. Then
 * the block keeps its shape, still reads as a chart, and nothing shifts once data arrives.
 *
 * Tell the reader why it is empty and what fills it ("MRR appears once …"), not just "no data".
 * `compact` is for a sparkline: only the baseline and one short muted line.
 */
export const ChartEmptyState = ({
  title,
  description,
  action,
  error = false,
  onRetry,
  compact = false,
  className,
}: {
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
  /** The query failed: an error icon and copy in place of the empty one. */
  error?: boolean
  /** With `error`, a "Try again" button. */
  onRetry?: () => void
  compact?: boolean
  className?: string
}) => {
  if (compact) {
    return (
      <div className={cn('relative flex h-full w-full items-center justify-center', className)}>
        <div
          aria-hidden
          className="absolute inset-x-0 bottom-0 border-t border-dashed border-border"
        />
        <p className={cn('text-xs', error ? 'text-destructive' : 'text-muted-foreground')}>
          {error ? "Couldn't load data" : title}
        </p>
      </div>
    )
  }

  return (
    <div className={cn('relative h-full w-full overflow-hidden', className)}>
      {/* The ghost chart: grid lines and the zero baseline, faded out in the middle for the text. */}
      <div
        aria-hidden
        className="absolute inset-x-2 bottom-[30px] top-[10px] flex flex-col justify-between [mask-image:radial-gradient(ellipse_50%_60%_at_center,transparent_55%,black_100%)]"
      >
        <div className="border-t border-border" />
        <div className="border-t border-border" />
        <div className="border-t border-border" />
        <div className="border-t-2 border-dashed border-border" />
      </div>

      <div className="relative flex h-full flex-col items-center justify-center gap-1.5 px-6 pb-[20px] text-center">
        {error && <AlertCircleIcon className="mb-0.5 size-5 text-destructive" aria-hidden />}
        <p className="text-sm font-medium text-foreground">
          {error ? "Couldn't load this chart" : title}
        </p>
        <p className="max-w-sm text-[13px] text-muted-foreground">
          {error ? 'Something went wrong fetching the data.' : description}
        </p>
        {error && onRetry ? (
          <Button variant="secondary" size="sm" className="mt-1.5 gap-1.5" onClick={onRetry}>
            <RotateCwIcon className="size-3.5" />
            Try again
          </Button>
        ) : (
          !error && action && <div className="pt-1.5">{action}</div>
        )}
      </div>
    </div>
  )
}
