import {
  PAY_AS_YOU_GO,
  headlineSuffix,
  isPayAsYouGo,
  planListAmount,
  planPriceAmount,
} from '../planPrice'
import { Mono } from '../primitives'

import type { PlanPrice } from '@/rpc/portal/subscription/v1/subscription_pb'

/** The plan's net price per cadence, the list price struck through when discounted. */
export const PlanPriceHeadline = ({
  price,
  size,
  suffixColor = 'var(--mtp-text-2)',
}: {
  price: PlanPrice
  size: number
  suffixColor?: string
}) => {
  if (isPayAsYouGo(price)) {
    return (
      <span style={{ fontSize: Math.round(size * 0.75), fontWeight: 600, letterSpacing: '-0.02em' }}>
        {PAY_AS_YOU_GO}
      </span>
    )
  }
  const list = planListAmount(price)
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', gap: 8 }}>
      <Mono style={{ fontSize: size, fontWeight: 600, letterSpacing: '-0.03em' }}>
        {planPriceAmount(price)}
      </Mono>
      {list && (
        <Mono
          style={{
            fontSize: Math.round(size * 0.5),
            color: 'var(--mtp-text-3)',
            textDecoration: 'line-through',
          }}
        >
          <span style={{ position: 'absolute', width: 1, height: 1, overflow: 'hidden', clip: 'rect(0 0 0 0)' }}>
            was{' '}
          </span>
          {list}
        </Mono>
      )}
      <span style={{ fontSize: Math.round(size * 0.42), color: suffixColor }}>
        {headlineSuffix(price)}
      </span>
    </div>
  )
}
