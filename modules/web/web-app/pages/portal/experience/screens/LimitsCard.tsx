import { compactNumber, decimalStr } from '../format'
import {
  groupLabel,
  limitNoun,
  limitOverGroups,
  limitPct,
  limitSubject,
  limitWindow,
} from '../limits'
import { Card, Eyebrow, Meter, Mono } from '../primitives'
import { boundaryDay, boundaryMonth, dayTime } from '../walletFormat'

import type { UsageLimitStatus } from '@/rpc/portal/customer/v1/models_pb'

const Resets = ({ limit }: { limit: UsageLimitStatus }) =>
  limit.resetsAt ? (
    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', whiteSpace: 'nowrap' }}>
      resets {boundaryDay(limit.resetsAt)}
    </span>
  ) : null

const LimitRows = ({ limit }: { limit: UsageLimitStatus }) => {
  const over = limitOverGroups(limit)
  return (
  <>
    {limit.groups.map(usage => {
      const label = groupLabel(usage, limit.name)
      const pct = limitPct(usage.used, limit.limit)
      const reached = over.includes(usage)
      return (
        <div
          key={label}
          style={{ display: 'flex', flexDirection: 'column', gap: 6 }}
        >
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, fontSize: 13 }}>
            <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{label}</span>
            <Mono style={{ whiteSpace: 'nowrap' }}>
              {compactNumber(usage.used)} / {compactNumber(limit.limit)}
            </Mono>
          </div>
          <Meter
            pct={pct}
            height={6}
            tone={reached ? 'danger' : 'default'}
            label={`${label} at ${pct}%`}
          />
        </div>
      )
    })}
  </>
  )
}

/** The usage caps on the account, per group (e.g. per API key) when they are split. */
export const LimitsCard = ({ limits }: { limits: UsageLimitStatus[] }) => {
  if (limits.length === 0) return null
  const single = limits.length === 1 ? limits[0] : undefined
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8 }}>
        <Eyebrow>{single ? `Limits · ${limitSubject(single)}` : 'Limits'}</Eyebrow>
        {single && <Resets limit={single} />}
      </div>
      {single ? (
        <LimitRows limit={single} />
      ) : (
        limits.map(limit => (
          <div key={limit.id} style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8 }}>
              <span style={{ fontSize: 12.5, fontWeight: 600, color: 'var(--mtp-text-2)' }}>
                {limitSubject(limit)}
              </span>
              <Resets limit={limit} />
            </div>
            <LimitRows limit={limit} />
          </div>
        ))
      )}
    </Card>
  )
}

/** The plan's own cap (e.g. Free's monthly tokens), shown in place of the next payment. */
export const PlanLimitCard = ({ limit }: { limit: UsageLimitStatus }) => {
  const used = limit.groups[0]?.used ?? '0'
  const pct = limitPct(used, limit.limit)
  const month = limitWindow(limit) === 'Monthly' ? boundaryMonth(limit.windowStart) : ''
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8 }}>
        <Eyebrow>
          {limit.name}
          {month ? ` · ${month}` : ''}
        </Eyebrow>
        <Resets limit={limit} />
      </div>
      <Mono style={{ fontSize: 30, fontWeight: 600, letterSpacing: '-0.02em' }}>
        {compactNumber(used)}{' '}
        <span style={{ fontSize: 16, color: 'var(--mtp-text-2)' }}>/ {compactNumber(limit.limit)}</span>
      </Mono>
      <Meter
        pct={pct}
        height={10}
        tone={limit.reached ? 'warning' : 'default'}
        label={`${decimalStr(used)} of ${decimalStr(limit.limit)} ${limitNoun(limit)} used`}
      />
      {(limit.breakdown.length > 0 || limit.reached) && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 13, color: 'var(--mtp-text-2)' }}>
          {limit.breakdown.map(row => {
            const label = groupLabel(row, limit.name)
            return (
              <div key={label} style={{ display: 'flex', justifyContent: 'space-between', gap: 12 }}>
                <span>{label}</span>
                <Mono>{decimalStr(row.used)}</Mono>
              </div>
            )
          })}
          {limit.reached && (
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, color: 'var(--mtp-warn-text)' }}>
              <span>Limit reached</span>
              {limit.reachedAt && <span>{dayTime(limit.reachedAt)}</span>}
            </div>
          )}
        </div>
      )}
    </Card>
  )
}
