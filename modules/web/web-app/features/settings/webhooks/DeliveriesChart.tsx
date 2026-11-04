import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@md/ui'

import {
  type DeliveryOutcomes,
  outcomesOf,
  outcomesTotal,
  successRate,
} from '@/features/settings/webhooks/endpointState'
import { type WebhookDailyStats } from '@/rpc/api/webhooksout/v1/webhooksout_pb'

const DAYS = 7
const BAR_AREA_PX = 96

interface Day extends DeliveryOutcomes {
  key: string
  label: string
}

/** Stacked top to bottom; retrying and cancelled only appear in the legend when present. */
const SEGMENTS: { key: keyof DeliveryOutcomes; label: string; color: string }[] = [
  { key: 'cancelled', label: 'Cancelled', color: 'bg-muted-foreground/40' },
  { key: 'failed', label: 'Failed', color: 'bg-destructive' },
  { key: 'retrying', label: 'Retrying', color: 'bg-warning' },
  { key: 'succeeded', label: 'Succeeded', color: 'bg-success' },
]
const ALWAYS_IN_LEGEND: (keyof DeliveryOutcomes)[] = ['succeeded', 'failed']

/** The last 7 calendar days, oldest first, with zero for days without deliveries. */
const lastDays = (stats: WebhookDailyStats[]): Day[] => {
  const byDay = new Map(stats.map(s => [s.day, s]))
  return Array.from({ length: DAYS }, (_, index) => {
    const date = new Date()
    date.setUTCDate(date.getUTCDate() - (DAYS - 1 - index))
    const key = date.toISOString().slice(0, 10)
    const found = byDay.get(key)
    return {
      key,
      label: date.toLocaleDateString('en-US', { weekday: 'short', timeZone: 'UTC' }),
      ...(found ? outcomesOf(found) : { succeeded: 0, failed: 0, retrying: 0, cancelled: 0 }),
    }
  })
}

const summary = (totals: DeliveryOutcomes): string => {
  const total = outcomesTotal(totals)
  if (total === 0) return 'No deliveries in the last 7 days'
  const parts = [`${total.toLocaleString()} deliveries`]
  if (totals.failed > 0) parts.push(`${totals.failed.toLocaleString()} failed`)
  if (totals.retrying > 0) parts.push(`${totals.retrying.toLocaleString()} retrying`)
  if (totals.cancelled > 0) parts.push(`${totals.cancelled.toLocaleString()} cancelled`)
  return `succeeded · ${parts.join(', ')}`
}

/** Headline success rate over 7 days, with daily bars by outcome below it. */
export const DeliveriesChart = ({ stats }: { stats: WebhookDailyStats[] }) => {
  const days = lastDays(stats)
  const totals: DeliveryOutcomes = {
    succeeded: days.reduce((sum, d) => sum + d.succeeded, 0),
    failed: days.reduce((sum, d) => sum + d.failed, 0),
    retrying: days.reduce((sum, d) => sum + d.retrying, 0),
    cancelled: days.reduce((sum, d) => sum + d.cancelled, 0),
  }
  const max = Math.max(1, ...days.map(outcomesTotal))
  const rate = successRate(totals)
  const legend = [...SEGMENTS]
    .reverse()
    .filter(s => ALWAYS_IN_LEGEND.includes(s.key) || totals[s.key] > 0)

  return (
    <div className="space-y-4">
      <div className="flex items-end justify-between gap-4">
        <div>
          <div className="text-2xl font-semibold">{rate ?? '–'}</div>
          <div className="text-xs text-muted-foreground">{summary(totals)}</div>
        </div>
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          {legend.map(segment => (
            <span key={segment.key} className="flex items-center gap-1.5">
              <span className={`h-2 w-2 rounded-sm ${segment.color}`} /> {segment.label}
            </span>
          ))}
        </div>
      </div>

      <TooltipProvider delayDuration={50}>
        <div className="flex items-end gap-2" style={{ height: BAR_AREA_PX + 20 }}>
          {days.map(day => {
            const bars = SEGMENTS.map(segment => ({
              ...segment,
              px: Math.round((day[segment.key] / max) * BAR_AREA_PX),
            })).filter(bar => day[bar.key] > 0)
            return (
              <Tooltip key={day.key}>
                <TooltipTrigger asChild>
                  <div className="flex h-full flex-1 cursor-default flex-col items-center justify-end gap-1">
                    <div className="flex w-full max-w-[28px] flex-col justify-end gap-[2px]">
                      {bars.map((bar, index) => (
                        <div
                          key={bar.key}
                          className={`w-full ${bar.color} ${index === 0 ? 'rounded-t-[4px]' : ''}`}
                          style={{ height: Math.max(bar.px, 2) }}
                        />
                      ))}
                      {bars.length === 0 && <div className="h-px w-full bg-border" />}
                    </div>
                    <span className="text-[11px] text-muted-foreground">{day.label}</span>
                  </div>
                </TooltipTrigger>
                <TooltipContent>
                  <div className="text-xs">
                    <div className="font-medium">{day.key}</div>
                    {[...SEGMENTS].reverse().map(segment =>
                      ALWAYS_IN_LEGEND.includes(segment.key) || day[segment.key] > 0 ? (
                        <div key={segment.key}>
                          {day[segment.key].toLocaleString()} {segment.label.toLowerCase()}
                        </div>
                      ) : null
                    )}
                  </div>
                </TooltipContent>
              </Tooltip>
            )
          })}
        </div>
      </TooltipProvider>
    </div>
  )
}
