import { Mono } from '../primitives'
import { creditGroupDetail } from '../walletFormat'

import { UsageFilterTag } from './UsageFilterTag'

import type { Wallet } from '@/rpc/portal/wallets/v1/models_pb'

/** The wallet's credit by kind, each with its own amount, expiry and what it pays. */
export const CreditGroups = ({ wallet }: { wallet: Wallet }) => {
  if (wallet.groups.length === 0) return null
  return (
    <ul
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      {wallet.groups.map(group => {
        const detail = creditGroupDetail(group, wallet)
        return (
          <li
            key={`${group.kind}-${group.subscriptionId ?? 'any'}-${group.usageOnly}-${group.productIds.join(',')}`}
            style={{
              display: 'flex',
              justifyContent: 'space-between',
              alignItems: 'baseline',
              gap: 12,
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
              <span
                style={{ display: 'inline-flex', alignItems: 'baseline', gap: 8, fontSize: 13.5 }}
              >
                <span style={{ fontWeight: 500 }}>{group.label}</span>
                <UsageFilterTag subscriptionId={group.subscriptionId} />
              </span>
              <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
                {detail.notes.join(' · ')}
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end' }}>
              <Mono style={{ fontWeight: 600, whiteSpace: 'nowrap' }}>{detail.amount}</Mono>
              {detail.scheduled && (
                <span style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>{detail.scheduled}</span>
              )}
            </div>
          </li>
        )
      })}
    </ul>
  )
}
