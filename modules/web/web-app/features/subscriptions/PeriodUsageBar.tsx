import { formatWalletAmount } from '@/features/credit/format'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency } from '@/lib/utils/numbers'

import type { WalletDenomination } from '@/features/credit/format'

/** Diagonal stripes in a theme token, for the projected part of a bar. */
export const hatch = (token: 'brand' | 'warning') =>
  `repeating-linear-gradient(135deg, hsl(var(--${token}) / 0.5) 0 4px, transparent 4px 8px)`

/** A bar filled to `usedPct`, then hatched up to `projectedPct` (all 0–100); `addedPct` tints
 * the end of the track. */
export const HatchedBar = ({
  usedPct,
  projectedPct,
  addedPct = 0,
  tone,
  label,
}: {
  usedPct: number
  projectedPct: number
  addedPct?: number
  tone: 'brand' | 'warning'
  label: string
}) => (
  <div
    role="img"
    aria-label={label}
    className="relative h-3 w-full overflow-hidden rounded-full bg-muted"
  >
    {addedPct > 0 && (
      <div className="absolute inset-y-0 right-0 bg-brand/25" style={{ width: `${addedPct}%` }} />
    )}
    <div
      className="absolute inset-y-0 left-0 rounded-full bg-brand"
      style={{ width: `${usedPct}%` }}
    />
    {projectedPct > usedPct && (
      <div
        className="absolute inset-y-0"
        style={{
          left: `${usedPct}%`,
          width: `${projectedPct - usedPct}%`,
          backgroundImage: hatch(tone),
        }}
      />
    )}
  </div>
)

/** "≈ $460": rounded to a whole unit of the denomination. */
export const formatApprox = (minor: bigint, denomination: WalletDenomination) => {
  const precision = denomination.creditUnit?.precision ?? currencyPrecision(denomination.currency)
  const scale = 10n ** BigInt(precision)
  const rounded = ((minor + scale / 2n) / scale) * scale
  return denomination.creditUnit
    ? formatWalletAmount(rounded, denomination)
    : formatCurrency(rounded, denomination.currency, { trimWholeAmounts: true })
}

/** The server's projection of a subscription's usage to its period end, and what it says of it. */
export type UsageProjection = {
  projected: bigint
  /** "covered by credit", "≈ $40 beyond credit, billed", … */
  outcome: string
  beyond: boolean
  /** Absent when the subscriptions summed end their periods on different days. */
  periodEnd?: string
}

/** The share used of the credit granted this period, `added` of it beyond the plan's. */
export type PeriodUtilization = { total: bigint; label: string; added: bigint }

/** The period's usage against the credit and its projection, under a one-line summary. */
export const PeriodUsageBar = ({
  available,
  used,
  projection,
  denomination,
  resetsOn,
  note,
  utilization,
}: {
  available: bigint
  used: bigint
  projection?: UsageProjection
  denomination: WalletDenomination
  /** The period's end, when every subscription summed shares it. */
  resetsOn?: string
  note?: string
  /** Measures the bar against the credit granted this period instead of `available`. */
  utilization?: PeriodUtilization
}) => {
  const amount = (minor: bigint) => formatWalletAmount(minor, denomination)
  const projected = projection?.projected
  const beyond = Boolean(projection?.beyond)
  const scale = utilization?.total ?? available

  const pct = (minor: bigint) =>
    scale > 0n ? Math.min(100, (Number(minor) / Number(scale)) * 100) : 0
  const usedPct = pct(used)
  const projectedPct = projected !== undefined ? pct(projected) : usedPct
  const projectionLabel = projection
    ? `about ${formatApprox(projection.projected, denomination)} by ${projection.periodEnd ? formatShortDay(projection.periodEnd) : 'period end'}, ${projection.outcome}`
    : undefined

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>
          {[
            `Used this billing period${resetsOn ? ` (resets ${formatShortDay(resetsOn)})` : ''}`,
            note,
          ]
            .filter(Boolean)
            .join(' · ')}
        </span>
        <span className="tabular-nums">
          <span className="font-medium text-foreground">
            {utilization
              ? `${utilization.label} (${amount(used)} of ${amount(utilization.total)}) used`
              : `${amount(used)} used`}
          </span>
          {projectionLabel && (
            <span className={beyond ? 'text-warning' : undefined}> · {projectionLabel}</span>
          )}
        </span>
      </div>
      {scale > 0n && (
        <HatchedBar
          usedPct={usedPct}
          projectedPct={projectedPct}
          addedPct={utilization ? pct(utilization.added) : 0}
          tone={beyond ? 'warning' : 'brand'}
          label={`${amount(used)} of ${amount(scale)} credit used this period${
            utilization?.added ? `, ${amount(utilization.added)} of it added this period` : ''
          }${projectionLabel ? `; ${projectionLabel}` : ''}`}
        />
      )}
      {!!utilization?.added && (
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground tabular-nums">
          <span aria-hidden className="h-2 w-2 rounded-full bg-brand/25" />
          {amount(utilization.added)} added this period
        </span>
      )}
    </div>
  )
}
