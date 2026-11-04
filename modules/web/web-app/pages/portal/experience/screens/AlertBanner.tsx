import { AlertTriangle } from 'lucide-react'

import { Slot } from '@/edition'

import { limitReachedCopy } from '../limits'
import { PButton } from '../primitives'

import type { UsageLimitStatus } from '@/rpc/portal/customer/v1/models_pb'
import type { GetCreditOverviewResponse } from '@/rpc/portal/wallets/v1/wallets_pb'
import type { ReactNode } from 'react'

type Tone = 'warn' | 'danger'

const TONE: Record<Tone, { bg: string; ink: string }> = {
  warn: { bg: 'var(--mtp-warn-bg)', ink: 'var(--mtp-warn-text)' },
  danger: { bg: 'var(--mtp-danger-bg)', ink: 'var(--mtp-danger)' },
}

export const AlertBanner = ({
  tone,
  icon,
  title,
  text,
  action,
}: {
  tone: Tone
  icon: ReactNode
  title: string
  text: string
  action?: { label: string; onClick: () => void }
}) => (
  <div
    role="alert"
    style={{
      display: 'flex',
      flexWrap: 'wrap',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 12,
      padding: '16px 18px',
      borderRadius: 'var(--mtp-r-card)',
      background: TONE[tone].bg,
      border: `1px solid color-mix(in srgb, ${TONE[tone].ink} 35%, transparent)`,
    }}
  >
    <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start', flex: 1, minWidth: 220 }}>
      <span
        aria-hidden
        style={{ color: TONE[tone].ink, flex: 'none', marginTop: 1, display: 'flex' }}
      >
        {icon}
      </span>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
        <span style={{ fontSize: 14, fontWeight: 600, color: TONE[tone].ink }}>{title}</span>
        <span style={{ fontSize: 13, color: 'var(--mtp-text)' }}>{text}</span>
      </div>
    </div>
    {action && (
      <PButton size="sm" onClick={action.onClick}>
        {action.label}
      </PButton>
    )}
  </div>
)

/** The account states that need the customer: the banners modules contribute, then a hard limit reached. */
export const PortalAlerts = ({
  credit,
  limits,
  freePlan,
  onAddCredit,
  onUpgrade,
}: {
  credit?: GetCreditOverviewResponse
  limits?: UsageLimitStatus[]
  freePlan?: boolean
  onAddCredit: (walletId: string) => void
  onUpgrade?: () => void
}) => {
  const reached = limits?.find(limit => limit.hard && limit.reached)
  const reachedCopy = reached && limitReachedCopy(reached, !!freePlan)

  return (
    <>
      <Slot name="portal.alerts" credit={credit} onAddCredit={onAddCredit} />
      {reachedCopy && (
        <AlertBanner
          tone="warn"
          icon={<AlertTriangle size={20} />}
          title={reachedCopy.title}
          text={reachedCopy.text}
          action={onUpgrade ? { label: 'Upgrade', onClick: onUpgrade } : undefined}
        />
      )}
    </>
  )
}
