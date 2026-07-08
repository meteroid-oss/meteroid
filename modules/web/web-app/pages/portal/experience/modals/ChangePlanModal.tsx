import { useState } from 'react'
import { toast } from 'sonner'

import { ChangeDirection } from '@/rpc/portal/subscription/v1/subscription_pb'

import { date, feeLabel, money } from '../format'
import { useSubscription } from '../hooks'
import { Modal, ModalCloseButton, Mono, PButton, Spinner } from '../primitives'

import type { AvailablePlan } from '@/rpc/portal/subscription/v1/subscription_pb'

const directionLabel = (d: ChangeDirection) =>
  d === ChangeDirection.UPGRADE
    ? 'Upgrade'
    : d === ChangeDirection.DOWNGRADE
      ? 'Downgrade'
      : 'Switch'

const confirmLabel = (d: ChangeDirection) =>
  d === ChangeDirection.UPGRADE
    ? 'Confirm upgrade'
    : d === ChangeDirection.DOWNGRADE
      ? 'Schedule downgrade'
      : 'Switch plan'

export const ChangePlanModal = ({
  subscriptionId,
  onClose,
  onApplied,
}: {
  subscriptionId: string
  onClose: () => void
  onApplied: () => void
}) => {
  const [selected, setSelected] = useState<AvailablePlan | null>(null)

  const sm = useSubscription(subscriptionId, {
    loadPlans: true,
    selectedPlanVersionId: selected?.planVersionId,
  })

  const plans = sm.plans.data?.plans ?? []
  const currency = sm.subscription?.currency ?? 'USD'
  const preview = sm.preview

  const confirm = async () => {
    if (!selected) return
    try {
      const { redirected } = await sm.confirmPlan(selected.planVersionId)
      if (!redirected) {
        toast.success('Plan updated')
        onApplied()
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to change plan')
    }
  }

  const net = preview?.proration ? Number(preview.proration.netAmountCents) : 0
  const dueToday = net > 0 ? net : 0
  const credit = net < 0 ? -net : 0

  return (
    <Modal open onClose={onClose} maxWidth={860}>
      {/* Header */}
      <div
        style={{
          position: 'sticky',
          top: 0,
          background: 'var(--mtp-sheet)',
          padding: '22px 24px 18px',
          borderBottom: '1px solid var(--mtp-border)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          zIndex: 2,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
          <span style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
            Change your plan
          </span>
          <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
            {sm.subscription?.planName} · changes are prorated
          </span>
        </div>
        <ModalCloseButton onClose={onClose} />
      </div>

      {/* Plan grid */}
      {sm.plans.isLoading ? (
        <div style={{ padding: 48, display: 'flex', justifyContent: 'center' }}>
          <Spinner size={20} />
        </div>
      ) : sm.plans.isError ? (
        <div style={{ padding: '40px 24px', textAlign: 'center' }}>
          <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
            We couldn&apos;t load available plans. Please try again later.
          </p>
        </div>
      ) : plans.length <= 1 ? (
        <div style={{ padding: '40px 24px', textAlign: 'center', display: 'flex', flexDirection: 'column', gap: 6 }}>
          <span style={{ fontSize: 14, fontWeight: 600 }}>No other plans available</span>
          <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
            There are no other self-service plans to switch to right now. Reach out to your account
            manager to change your plan.
          </p>
        </div>
      ) : (
        <div
          style={{
            padding: '22px 24px',
            display: 'grid',
            gridTemplateColumns: `repeat(${Math.min(4, Math.max(1, plans.length))},1fr)`,
            gap: 11,
          }}
          className="mtp-plan-grid"
        >
          {plans.map(p => {
            const isSelected = selected?.planVersionId === p.planVersionId
            return (
              <button
                key={p.planVersionId}
                onClick={() => !p.isCurrent && setSelected(p)}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 9,
                  padding: '16px 14px',
                  borderRadius: 'var(--mtp-r-ctrl)',
                  cursor: p.isCurrent ? 'default' : 'pointer',
                  textAlign: 'left',
                  fontFamily: 'inherit',
                  background: isSelected ? 'var(--mtp-spot)' : 'var(--mtp-surface)',
                  color: isSelected ? 'var(--mtp-spot-text)' : 'var(--mtp-text)',
                  border: isSelected
                    ? '1.5px solid var(--mtp-accent-ink)'
                    : '1px solid var(--mtp-border)',
                  transition: 'background .14s, border-color .14s',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    minHeight: 18,
                  }}
                >
                  <span style={{ fontSize: 13.5, fontWeight: 600 }}>{p.planName}</span>
                  {p.isCurrent && (
                    <span
                      style={{
                        fontSize: 9.5,
                        fontWeight: 600,
                        padding: '1px 7px',
                        borderRadius: 20,
                        color: isSelected ? 'var(--mtp-spot-2)' : 'var(--mtp-text-2)',
                        background: isSelected ? 'var(--mtp-spot-track)' : 'var(--mtp-track)',
                      }}
                    >
                      Current
                    </span>
                  )}
                </div>
                <Mono style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
                  {p.headlineFee ? feeLabel(p.headlineFee, currency) : 'Custom'}
                </Mono>
                {p.description && (
                  <span
                    style={{
                      fontSize: 11.5,
                      color: isSelected ? 'var(--mtp-spot-2)' : 'var(--mtp-text-2)',
                      lineHeight: 1.35,
                    }}
                  >
                    {p.description}
                  </span>
                )}
              </button>
            )
          })}
        </div>
      )}

      {/* Proration summary */}
      {selected && (
        <div
          style={{
            margin: '0 24px 24px',
            border: '1px solid var(--mtp-border)',
            borderRadius: 'var(--mtp-r-ctrl)',
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              padding: '15px 18px',
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--mtp-surface-2)',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                {preview
                  ? `${directionLabel(preview.changeDirection)}${
                      preview.changeDirection === ChangeDirection.UPGRADE
                        ? ' — effective immediately'
                        : ` — effective ${date(preview.effectiveDate)}`
                    }`
                  : 'Calculating…'}
              </span>
              {sm.isPreviewLoading && <Spinner size={14} />}
            </div>
            <Line label="New plan" value={selected.headlineFee ? feeLabel(selected.headlineFee, currency) : 'Custom'} />
            {preview?.proration && preview.proration.daysInPeriod > 0 && (
              <Line
                label={`Prorated · ${preview.proration.daysRemaining}/${preview.proration.daysInPeriod} days`}
                value={money(preview.proration.netAmountCents, currency)}
                accent
              />
            )}
            {preview && preview.proration && Number(preview.proration.arrearsChargeCents) > 0 && (
              <Line
                label="Added to next invoice"
                value={money(preview.proration.arrearsChargeCents, currency)}
              />
            )}
          </div>
          <div
            style={{
              padding: '15px 18px',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              borderTop: '1px solid var(--mtp-border)',
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.3 }}>
              <span style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>
                {credit > 0 ? 'Credited to next invoice' : 'Due today'}
              </span>
              <Mono style={{ fontSize: 21, fontWeight: 600, letterSpacing: '-0.02em' }}>
                {money(credit > 0 ? credit : dueToday, currency)}
              </Mono>
            </div>
            <div style={{ display: 'flex', gap: 9 }}>
              <PButton variant="secondary" onClick={onClose}>
                Cancel
              </PButton>
              <PButton onClick={confirm} loading={sm.confirmPending} disabled={!preview}>
                {preview ? confirmLabel(preview.changeDirection) : 'Confirm'}
              </PButton>
            </div>
          </div>
        </div>
      )}
    </Modal>
  )
}

const Line = ({
  label,
  value,
  accent,
}: {
  label: string
  value: string
  accent?: boolean
}) => (
  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{label}</span>
    <Mono
      style={{
        fontSize: 12.5,
        fontWeight: 600,
        color: accent ? 'var(--mtp-accent-ink)' : 'var(--mtp-text)',
      }}
    >
      {value}
    </Mono>
  </div>
)
