import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { unitIsCurrency } from '@/features/plans/charges/kinds'
import { levelWords } from '@/features/subscriptions/capacityLevels'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { ChangeDirection } from '@/rpc/portal/subscription/v1/subscription_pb'

import { Contact } from '../Contact'
import { PeriodToggle } from '../PeriodToggle'
import { appliedMessage, appliesOn, feeLabel, money, packLineView } from '../format'
import { usePortalOverview, useSubscription } from '../hooks'
import { planPriceShort } from '../planPrice'
import { Modal, ModalCloseButton, Mono, PButton, Pill, Spinner } from '../primitives'
import {
  cheaperThanCurrent,
  isCurrentChoice,
  isPeriodSwitch,
  notOffered,
  notOfferedLabel,
  periodSwitchLabel,
  usePlanOption,
  variantFor,
  variantLabel,
} from '../variants'
import { customerError } from '../walletFormat'

import type {
  AvailablePlan,
  AvailableOption,
  PlanChangeLevels,
} from '@/rpc/portal/subscription/v1/subscription_pb'

type Selection = { plan: AvailablePlan; variant?: AvailableOption }

/** The usage-levels charges the preview of the plan and option `key` offers. */
type OfferedLevels = { key: string; charges: PlanChangeLevels[] }

const choicePriceLabel = (
  plan: AvailablePlan,
  variant: AvailableOption | undefined,
  currency: string
) =>
  (variant?.price ?? plan.price)
    ? planPriceShort((variant?.price ?? plan.price)!)
    : plan.headlineFee
      ? feeLabel(plan.headlineFee, currency)
      : 'Custom'

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
  const [selected, setSelectedPlan] = useState<Selection | null>(null)
  // The level picked per usage-levels charge of the plan moved to; the server's default otherwise.
  const [levels, setLevels] = useState<Record<string, bigint>>({})
  const setSelected = (next: Selection | null) => {
    setSelectedPlan(next)
    setLevels({})
  }

  const sm = useSubscription(subscriptionId, {
    loadPlans: true,
    selectedPlanVersionId: selected?.plan.planVersionId,
    selectedOption: selected?.variant?.id,
    selectedLevels: levels,
  })

  // The levels offered stay up while a pick re-previews the change.
  const selectionKey = selected
    ? `${selected.plan.planVersionId}:${selected.variant?.id ?? ''}`
    : ''
  const [offeredLevels, setOfferedLevels] = useState<OfferedLevels>({ key: '', charges: [] })
  useEffect(() => {
    if (sm.preview) setOfferedLevels({ key: selectionKey, charges: sm.previewLevels })
  }, [sm.preview, sm.previewLevels, selectionKey])
  const levelCharges = offeredLevels.key === selectionKey ? offeredLevels.charges : []

  const { overview } = usePortalOverview()
  const plans = sm.plans.data?.plans ?? []
  const selfServeDowngrade = sm.plans.data?.selfServeDowngrade ?? false
  const { choices, option, setOption, showToggle } = usePlanOption(
    plans,
    sm.plans.data?.periodSwitch ?? false
  )
  const currency = sm.subscription?.currency ?? 'USD'
  const preview = sm.preview
  const previewError = sm.previewError
    ? customerError(sm.previewError, 'We couldn’t calculate this change. Please try again.')
    : undefined
  const periodSwitch =
    selected?.variant && isPeriodSwitch(selected.plan, selected.variant)
      ? selected.variant
      : undefined
  const downgradeBlocked =
    !!preview &&
    preview.changeDirection === ChangeDirection.DOWNGRADE &&
    !selfServeDowngrade &&
    !selected?.plan.isCurrent

  const changeOption = (next: string) => {
    setOption(next)
    if (!selected) return
    const variant = variantFor(selected.plan, next)
    setSelected(
      isCurrentChoice(selected.plan, variant) || notOffered(selected.plan, variant)
        ? null
        : { plan: selected.plan, variant }
    )
  }

  const confirm = async () => {
    if (!selected || downgradeBlocked) return
    try {
      const { redirected, res } = await sm.confirmPlan(
        selected.plan.planVersionId,
        selected.variant?.id,
        levels
      )
      if (!redirected) {
        toast.success(appliedMessage('Plan updated', res.invoicePending))
        onApplied()
      }
    } catch (e) {
      toast.error(customerError(e, 'Unable to change plan'))
    }
  }

  const heading = preview
    ? `${
        periodSwitch ? periodSwitchLabel(periodSwitch) : directionLabel(preview.changeDirection)
      } — ${appliesOn(preview.effectiveDate)}`
    : (previewError ?? 'Calculating…')

  const proration = preview?.proration
  const net = proration ? Number(proration.netAmountCents) : 0
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
            {sm.subscription?.planName} · upgrades apply now, prorated; downgrades at the end of
            your period
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
      ) : plans.length <= 1 && !showToggle ? (
        <div
          style={{
            padding: '40px 24px',
            textAlign: 'center',
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
          }}
        >
          <span style={{ fontSize: 14, fontWeight: 600 }}>No other plans available</span>
          <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
            There are no other self-service plans to switch to right now. Reach out to your account
            manager to change your plan.
          </p>
        </div>
      ) : (
        <>
          {showToggle && (
            <div style={{ padding: '18px 24px 0', display: 'flex', justifyContent: 'center' }}>
              <PeriodToggle choices={choices} value={option} onChange={changeOption} />
            </div>
          )}
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
              const variant = variantFor(p, option)
              const current = isCurrentChoice(p, variant)
              const unavailable = notOffered(p, variant)
              const label = variantLabel(p, variant, showToggle ? option : undefined)
              const isSelected = selected?.plan.planVersionId === p.planVersionId
              const contactToDowngrade =
                !selfServeDowngrade && cheaperThanCurrent(plans, p, variant, option)
              return (
                <button
                  key={p.planVersionId}
                  type="button"
                  aria-pressed={isSelected}
                  disabled={current || unavailable}
                  onClick={() => setSelected({ plan: p, variant })}
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 9,
                    padding: '16px 14px',
                    borderRadius: 'var(--mtp-r-ctrl)',
                    cursor: current || unavailable ? 'default' : 'pointer',
                    opacity: unavailable ? 0.55 : 1,
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
                    {current && (
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
                  {label && (
                    <span
                      style={{
                        fontSize: 11,
                        fontWeight: 500,
                        marginTop: -5,
                        color: isSelected ? 'var(--mtp-spot-2)' : 'var(--mtp-text-3)',
                      }}
                    >
                      {label}
                    </span>
                  )}
                  <Mono style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
                    {unavailable
                      ? notOfferedLabel(choices.find(c => c.alias === option))
                      : choicePriceLabel(p, variant, currency)}
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
                  {variant && isPeriodSwitch(p, variant) && (
                    <span
                      style={{
                        marginTop: 'auto',
                        fontSize: 11.5,
                        fontWeight: 600,
                        color: isSelected ? 'var(--mtp-spot-text)' : 'var(--mtp-accent-ink)',
                      }}
                    >
                      {periodSwitchLabel(variant)}
                    </span>
                  )}
                  {contactToDowngrade && (
                    <span
                      style={{
                        marginTop: 'auto',
                        fontSize: 11.5,
                        color: isSelected ? 'var(--mtp-spot-2)' : 'var(--mtp-text-3)',
                      }}
                    >
                      Contact us to downgrade
                    </span>
                  )}
                </button>
              )
            })}
          </div>
        </>
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
              <span
                role={previewError ? 'alert' : undefined}
                style={{
                  fontSize: 12.5,
                  fontWeight: 600,
                  color: previewError ? 'var(--mtp-danger)' : undefined,
                }}
              >
                {heading}
              </span>
              {sm.isPreviewLoading && <Spinner size={14} />}
            </div>
            <Line
              label="New plan"
              hint={variantLabel(selected.plan, selected.variant)}
              value={choicePriceLabel(selected.plan, selected.variant, currency)}
            />
            {levelCharges.map(charge => (
              <LevelChoices
                key={charge.componentId}
                charge={charge}
                currency={currency}
                picked={levels[charge.componentId]}
                onPick={included =>
                  setLevels(prev => ({ ...prev, [charge.componentId]: included }))
                }
              />
            ))}
            {!downgradeBlocked && proration && (
              <>
                {proration.daysInPeriod > 0 && (
                  <Line
                    label={`Prorated · ${proration.daysRemaining}/${proration.daysInPeriod} days`}
                    value={money(proration.netAmountCents, currency)}
                    accent
                  />
                )}
                {proration.packChanges.map((line, idx) => {
                  const view = packLineView(line, currency)
                  return view ? (
                    <Line
                      key={`${line.kind}-${line.name}-${idx}`}
                      label={view.label}
                      hint={line.name}
                      value={view.value}
                    />
                  ) : null
                })}
                {Number(proration.arrearsChargeCents) > 0 && (
                  <Line
                    label="Added to next invoice"
                    value={money(proration.arrearsChargeCents, currency)}
                  />
                )}
              </>
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
            {downgradeBlocked ? (
              <span />
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.3 }}>
                <span style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>
                  {credit > 0 ? 'Credited to next invoice' : 'Due today'}
                </span>
                <Mono style={{ fontSize: 21, fontWeight: 600, letterSpacing: '-0.02em' }}>
                  {money(credit > 0 ? credit : dueToday, currency)}
                </Mono>
              </div>
            )}
            <div style={{ display: 'flex', gap: 9 }}>
              <PButton variant="secondary" onClick={onClose}>
                Cancel
              </PButton>
              {downgradeBlocked ? (
                <span
                  role="status"
                  style={{
                    alignSelf: 'center',
                    fontSize: 12.5,
                    fontWeight: 500,
                    color: 'var(--mtp-text-2)',
                  }}
                >
                  Downgrades are arranged by our team: please{' '}
                  <Contact
                    label="contact us"
                    email={overview?.invoicingEntityContactEmail}
                  />
                  .
                </span>
              ) : (
                <PButton onClick={confirm} loading={sm.confirmPending} disabled={!preview}>
                  {periodSwitch
                    ? periodSwitchLabel(periodSwitch)
                    : preview
                      ? confirmLabel(preview.changeDirection)
                      : 'Confirm'}
                </PButton>
              )}
            </div>
          </div>
        </div>
      )}
    </Modal>
  )
}

/**
 * A usage-levels charge of the plan moved to, sold at several levels: the customer picks the one
 * the change lands on, each worded as the level-change modal words it.
 */
const LevelChoices = ({
  charge,
  currency,
  picked,
  onPick,
}: {
  charge: PlanChangeLevels
  currency: string
  /** The customer's pick; unset: the level the preview lands on. */
  picked?: bigint
  onPick: (included: bigint) => void
}) => {
  const unit = {
    money: unitIsCurrency({ plural: charge.unit ?? null }, currency),
    plural: charge.unit,
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{charge.name}</span>
      {charge.levels.map(level => {
        const held = charge.held !== undefined && level.included === charge.held
        const chosen = picked !== undefined ? level.included === picked : level.current
        return (
          <button
            key={level.priceId}
            type="button"
            aria-pressed={chosen}
            onClick={() => onPick(level.included)}
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 10,
              padding: '9px 12px',
              borderRadius: 'var(--mtp-r-ctrl)',
              border: chosen ? '1px solid var(--mtp-accent-ink)' : '1px solid var(--mtp-border)',
              background: chosen ? 'var(--mtp-accent-weak)' : 'var(--mtp-surface)',
              color: 'var(--mtp-text)',
              fontFamily: 'inherit',
              fontSize: 12.5,
              textAlign: 'left',
              cursor: 'pointer',
            }}
          >
            <span>
              {levelWords(
                { included: level.included, rate: level.fee?.amount ?? '0' },
                currency,
                level.fee?.cadence ?? SubscriptionFeeBillingPeriod.MONTHLY,
                unit
              )}
            </span>
            {held && <Pill badge={{ label: 'Current', tone: 'ok' }} />}
          </button>
        )
      })}
    </div>
  )
}

const Line = ({
  label,
  hint,
  value,
  accent,
}: {
  label: string
  hint?: string
  value: string
  accent?: boolean
}) => (
  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
    <span style={{ display: 'flex', flexDirection: 'column', gap: 1, minWidth: 0 }}>
      <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{label}</span>
      {hint && <span style={{ fontSize: 11.5, color: 'var(--mtp-text-3)' }}>{hint}</span>}
    </span>
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
