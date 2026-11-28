import { ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { unitIsCurrency } from '@/features/plans/charges/kinds'
import { levelSize, levelWords } from '@/features/subscriptions/capacityLevels'
import { useQuery } from '@/lib/connectrpc'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  previewAddOnQuantity,
  previewLevelChange,
  previewSeats,
} from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'
import { AdjustmentStatus, PendingEventType } from '@/rpc/portal/subscription/v1/subscription_pb'

import { UsageChart } from '../UsageChart'
import { usePortalNav } from '../context'
import {
  appliedMessage,
  date,
  feeLabel,
  increasesCaption,
  money,
  periodSuffix,
  subStatusBadge,
} from '../format'
import { useSubscription } from '../hooks'
import {
  Card,
  CenterState,
  Divider,
  LinkButton,
  Modal,
  ModalCloseButton,
  Mono,
  PanelCard,
  PButton,
  Pill,
  Spinner,
  Stepper,
} from '../primitives'
import { boundaryDay } from '../walletFormat'

import { CancelSubscription } from './CancelSubscription'

import type { LevelUnit } from '@/features/subscriptions/capacityLevels'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'
import type {
  AdjustableComponent,
  AvailableAddOn,
  CapacityLevel,
  ComponentFee,
  SubscriptionDetails,
} from '@/rpc/portal/subscription/v1/subscription_pb'

type SubTab = 'overview' | 'addons' | 'preview'

const SUBTABS: { key: SubTab; label: string }[] = [
  { key: 'overview', label: 'Overview' },
  { key: 'addons', label: 'Add-ons' },
  { key: 'preview', label: 'Preview' },
]

const eventLabel = (t: PendingEventType) =>
  t === PendingEventType.CANCEL
    ? 'Cancellation'
    : t === PendingEventType.AMENDMENT
      ? 'Plan amendment'
      : 'Plan change'

export const SubscriptionDetailScreen = ({
  subscriptionId,
  onBack,
}: {
  subscriptionId: string
  onBack: () => void
}) => {
  const { openChangePlan, currency: portalCurrency } = usePortalNav()
  const [tab, setTab] = useState<SubTab>('overview')
  const [cancelOpen, setCancelOpen] = useState(false)

  const sm = useSubscription(subscriptionId)
  const sub = sm.subscription

  if (sm.details.isLoading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: 48 }}>
        <Spinner size={20} />
      </div>
    )
  }
  if (!sub) {
    return <CenterState title="Subscription not found" hint="It may have been removed." />
  }

  const currency = sub.currency || portalCurrency
  const headline = feeLabel(sub.headlineFee, currency)
  const pending = sub.pendingEvent
  const isCancelScheduled = pending?.eventType === PendingEventType.CANCEL

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <LinkButton style={{ color: 'var(--mtp-text-2)', alignSelf: 'flex-start' }} onClick={onBack}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M15 18l-6-6 6-6" />
        </svg>
        All subscriptions
      </LinkButton>

      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap' }}>
        <div
          style={{
            width: 46,
            height: 46,
            borderRadius: 'var(--mtp-r-ctrl)',
            background: 'var(--mtp-accent-weak)',
            color: 'var(--mtp-accent-ink)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            flex: '0 0 46px',
          }}
        >
          <svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9">
            <path d="M4 7h16M4 12h16M4 17h10" />
          </svg>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0, flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ fontSize: 19, fontWeight: 600, letterSpacing: '-0.02em' }}>
              {sub.planName}
            </span>
            <Pill badge={subStatusBadge(sub.status)} dot />
          </div>
          <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>
            {headline}
            {isCancelScheduled && pending?.scheduledDate ? (
              <>
                {' · '}
                <span style={{ color: 'var(--mtp-danger)' }}>cancels {date(pending.scheduledDate)}</span>
              </>
            ) : sub.currentPeriodEnd ? (
              ` · renews ${date(sub.currentPeriodEnd)}`
            ) : (
              ''
            )}
          </span>
        </div>
        <div style={{ display: 'flex', gap: 9 }}>
          {sub.canChangePlan && (
            <PButton onClick={() => openChangePlan(subscriptionId)}>Change plan</PButton>
          )}
          {!isCancelScheduled && (
            <PButton variant="danger" onClick={() => setCancelOpen(true)}>
              Cancel
            </PButton>
          )}
        </div>
      </div>

      {/* Pending change banner */}
      {pending && (
        <Card
          pad="14px 18px"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            background: 'var(--mtp-surface-2)',
          }}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="var(--mtp-text-2)" strokeWidth="1.9">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 7v5l3 2" />
          </svg>
          <span style={{ fontSize: 12.5, color: 'var(--mtp-text)' }}>
            {eventLabel(pending.eventType)} scheduled
            {pending.newPlanName ? ` → ${pending.newPlanName}` : ''} for{' '}
            <strong>{date(pending.scheduledDate)}</strong>
          </span>
          {pending.customerCancellable && (
            <LinkButton
              style={{ marginLeft: 'auto', color: 'var(--mtp-danger)' }}
              onClick={async () => {
                try {
                  await sm.cancelScheduled(pending.id)
                  toast.success(isCancelScheduled ? 'Cancellation undone' : 'Scheduled change canceled')
                } catch (e) {
                  toast.error(e instanceof Error ? e.message : 'Unable to cancel')
                }
              }}
            >
              {sm.cancelPending
                ? 'Canceling…'
                : isCancelScheduled
                  ? 'Undo cancellation'
                  : 'Cancel this change'}
            </LinkButton>
          )}
        </Card>
      )}

      {/* Sub-tabs */}
      <div style={{ display: 'flex', gap: 24, borderBottom: '1px solid var(--mtp-border)' }}>
        {SUBTABS.map(t => {
          const active = tab === t.key
          return (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              className="mtp-tab"
              style={{
                padding: '10px 0',
                fontSize: 13,
                fontWeight: 500,
                fontFamily: 'inherit',
                cursor: 'pointer',
                background: 'none',
                border: 'none',
                borderBottom: active ? '2px solid var(--mtp-accent-ink)' : '2px solid transparent',
                color: active ? 'var(--mtp-text)' : 'var(--mtp-text-2)',
                marginBottom: -1,
              }}
            >
              {t.label}
            </button>
          )
        })}
      </div>

      {tab === 'overview' && <OverviewTab sub={sub} currency={currency} sm={sm} />}
      {tab === 'addons' && <AddOnsTab addOns={sm.addOns} currency={currency} sm={sm} />}
      {tab === 'preview' && (
        <PreviewTab sm={sm} currency={currency} subscriptionId={subscriptionId} />
      )}

      <CancelSubscription
        open={cancelOpen}
        onClose={() => setCancelOpen(false)}
        onDone={() => setCancelOpen(false)}
        subscriptionId={subscriptionId}
        planName={sub.planName}
        periodEnd={sub.currentPeriodEnd}
      />
    </div>
  )
}

const Row = ({ label, value, last }: { label: string; value: React.ReactNode; last?: boolean }) => (
  <div
    style={{
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'center',
      padding: '11px 0',
      borderBottom: last ? 'none' : '1px solid var(--mtp-border)',
    }}
  >
    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{label}</span>
    <Mono style={{ fontSize: 12.5, fontWeight: 500 }}>{value}</Mono>
  </div>
)

const OverviewTab = ({
  sub,
  currency,
  sm,
}: {
  sub: SubscriptionDetails
  currency: string
  sm: ReturnType<typeof useSubscription>
}) => {
  const inv = sm.upcomingInvoice
  // Credit other pending invoices share is an estimate, and so is what is left to pay.
  const approx = inv?.creditEstimateShared ? '≈ ' : ''
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }} className="mtp-two-col">
      <Card>
        <span style={{ fontSize: 13.5, fontWeight: 600 }}>Plan summary</span>
        <div style={{ marginTop: 10 }}>
          <Row label="Plan" value={sub.planName} />
          <Row label="Price" value={feeLabel(sub.headlineFee, currency)} />
          <Row label="Status" value={subStatusBadge(sub.status).label} />
          <Row
            label="Renews"
            value={sub.currentPeriodEnd ? date(sub.currentPeriodEnd) : '—'}
            last
          />
        </div>
      </Card>
      <Card>
        <span style={{ fontSize: 13.5, fontWeight: 600 }}>Next invoice</span>
        {sm.isUpcomingLoading ? (
          <div style={{ padding: 16 }}>
            <Spinner size={16} />
          </div>
        ) : sm.isUpcomingError ? (
          <p style={{ fontSize: 12.5, color: 'var(--mtp-text-3)', marginTop: 12 }}>
            The next invoice is unavailable right now.
          </p>
        ) : inv ? (
          <div style={{ marginTop: 10 }}>
            <Row label="Period" value={`${date(inv.periodStart)} – ${date(inv.periodEnd)}`} />
            <Row label="Subtotal" value={money(inv.subtotal, inv.currency)} />
            {inv.discount > 0n && <Row label="Discount" value={`−${money(inv.discount, inv.currency)}`} />}
            {inv.promotionalCredits > 0n && (
              <Row
                label="Free credit"
                value={`${approx}−${money(inv.promotionalCredits, inv.currency)}`}
              />
            )}
            <Row label="Tax" value={money(inv.taxAmount, inv.currency)} />
            {/* The amount due is the total less the credit applied, never below zero. */}
            {inv.total > inv.amountDue && (
              <Row
                label="Paid with credit"
                value={`${approx}−${money(inv.total - inv.amountDue, inv.currency)}`}
              />
            )}
            <Row label="Total due" value={`${approx}${money(inv.amountDue, inv.currency)}`} last />
            {inv.creditEstimateShared && inv.billedFirst.length > 0 && (
              <p style={{ fontSize: 12, color: 'var(--mtp-text-3)', margin: '10px 0 0' }}>
                Your credit is shared:{' '}
                {inv.billedFirst
                  .map(
                    first =>
                      `${first.planName ?? 'an earlier invoice'} bills first, on ${boundaryDay(first.invoiceDate)}`
                  )
                  .join(' · ')}
              </p>
            )}
          </div>
        ) : (
          <p style={{ fontSize: 12.5, color: 'var(--mtp-text-3)', marginTop: 12 }}>
            No upcoming invoice.
          </p>
        )}
      </Card>
      </div>
      <CommitmentsCard sm={sm} currency={currency} />
    </div>
  )
}

const ManagedNote = ({ children }: { children: React.ReactNode }) => (
  <div
    style={{
      padding: '12px 20px',
      fontSize: 12,
      color: 'var(--mtp-text-2)',
      background: 'var(--mtp-surface-2)',
      borderTop: '1px solid var(--mtp-border)',
    }}
  >
    {children}
  </div>
)

const AddOnsTab = ({
  addOns,
  currency,
  sm,
}: {
  addOns: AvailableAddOn[]
  currency: string
  sm: ReturnType<typeof useSubscription>
}) => {
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<{ a: AvailableAddOn; target: number } | null>(null)
  const canEdit = sm.canSelfServe

  if (addOns.length === 0) {
    return (
      <CenterState
        title="No add-ons available"
        hint="There are no add-ons offered on this plan right now."
      />
    )
  }

  // Attach a brand-new add-on. Free/usage-billed add-ons are added instantly;
  // paid ones redirect to checkout (handled inside sm.purchase).
  const addAddOn = async (a: AvailableAddOn) => {
    setBusyId(a.addOnId)
    try {
      const { redirected, res } = await sm.purchase(a.addOnId)
      if (!redirected) {
        toast.success(appliedMessage(`${a.name} added`, res.invoicePending))
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to add add-on')
    } finally {
      setBusyId(null)
    }
  }

  // Increases apply immediately (prorated); decreases/removals schedule to period end.
  const setQty = async (a: AvailableAddOn, target: number) => {
    setBusyId(a.addOnId)
    try {
      const { redirected, res } = await sm.setAddOnQuantity(a.addOnId, target)
      if (!redirected) {
        if (res.status === AdjustmentStatus.ADJUSTMENT_SCHEDULED) {
          toast.success(
            res.effectiveDate
              ? `${a.name} reduces on ${date(res.effectiveDate)}`
              : 'Change scheduled for period end'
          )
        } else {
          toast.success(
            target === 0
              ? `${a.name} removed`
              : appliedMessage(`${a.name} updated`, res.invoicePending)
          )
        }
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to update add-on')
    } finally {
      setBusyId(null)
    }
  }

  return (
    <PanelCard
      title="Add-ons"
      action={
        <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
          {increasesCaption(sm.invoicesReviewed)}
        </span>
      }
    >
      {addOns.map((a, i) => {
        const busy = busyId === a.addOnId
        return (
          <div
            key={a.addOnId}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 14,
              padding: '15px 20px',
              borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0, flex: 1 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: 13.5, fontWeight: 500 }}>{a.name}</span>
                {a.currentQuantity > 0 && (
                  <Pill badge={{ label: `×${a.currentQuantity}`, tone: 'ok' }} />
                )}
              </div>
              {a.description && (
                <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>{a.description}</span>
              )}
            </div>
            <Mono style={{ fontSize: 13, color: 'var(--mtp-text)' }}>{feeLabel(a.fee, currency)}</Mono>
            {a.currentQuantity > 0 ? (
              <Stepper
                value={a.currentQuantity}
                min={0}
                max={a.maxInstances ?? undefined}
                disabled={!canEdit}
                busy={busy}
                onChange={next => setConfirm({ a, target: next })}
              />
            ) : (
              <PButton
                size="sm"
                disabled={!canEdit}
                loading={busy}
                onClick={() => addAddOn(a)}
              >
                Add
              </PButton>
            )}
          </div>
        )
      })}
      {!canEdit && (
        <ManagedNote>
          Add-ons on this subscription are managed by your account team — contact them to make
          changes.
        </ManagedNote>
      )}
      <QuantityChangeModal
        change={
          confirm
            ? {
                name: confirm.a.name,
                fee: confirm.a.fee,
                current: confirm.a.currentQuantity,
                target: confirm.target,
                subscriptionId: sm.subscription?.id ?? '',
                kind: 'addon',
                targetId: confirm.a.addOnId,
              }
            : null
        }
        currency={currency}
        periodEnd={sm.subscription?.currentPeriodEnd}
        busy={!!confirm && busyId === confirm.a.addOnId}
        onClose={() => setConfirm(null)}
        onConfirm={async () => {
          if (!confirm) return
          await setQty(confirm.a, confirm.target)
          setConfirm(null)
        }}
      />
    </PanelCard>
  )
}

const CommitmentsCard = ({
  sm,
  currency,
}: {
  sm: ReturnType<typeof useSubscription>
  currency: string
}) => {
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<{ c: AdjustableComponent; target: number } | null>(null)
  const [levelOf, setLevelOf] = useState<AdjustableComponent | null>(null)
  const components = sm.adjustableComponents
  if (components.length === 0) return null

  const setQty = async (c: AdjustableComponent, target: number) => {
    setBusyId(seatKey(c))
    try {
      const { redirected, res } = await sm.setSeats(c.componentId, target, c.addOnId)
      if (!redirected) {
        if (res.status === AdjustmentStatus.ADJUSTMENT_SCHEDULED) {
          toast.success(
            res.effectiveDate ? `Reduces on ${date(res.effectiveDate)}` : 'Scheduled for period end'
          )
        } else {
          toast.success(appliedMessage('Updated', res.invoicePending))
        }
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to update')
    } finally {
      setBusyId(null)
    }
  }

  return (
    <PanelCard
      title="Seats & commitments"
      action={
        <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
          {increasesCaption(sm.invoicesReviewed)}
        </span>
      }
    >
      {components.map((c, i) => {
        const isSeats = c.kind === 'seats'
        const current = Number(c.current)
        const min = c.min != null ? Number(c.min) : 0
        const max = c.max != null ? Number(c.max) : undefined
        return (
          <div
            key={seatKey(c)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 14,
              padding: '15px 20px',
              borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0, flex: 1 }}>
              <span style={{ fontSize: 13.5, fontWeight: 500 }}>{c.name}</span>
              <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
                {isSeats || !c.unitFee
                  ? `${feeLabel(c.unitFee, currency)}${c.unit ? ` · ${current} ${c.unit}` : ''}`
                  : levelWords(
                      { included: BigInt(c.current), rate: c.unitFee.amount },
                      currency,
                      c.unitFee.cadence,
                      capacityUnit(c, currency)
                    )}
              </span>
            </div>
            {!isSeats && c.levels.length > 1 ? (
              <PButton
                size="sm"
                variant="secondary"
                disabled={!sm.canSelfServe}
                loading={busyId === c.componentId}
                onClick={() => setLevelOf(c)}
              >
                Change level
              </PButton>
            ) : isSeats ? (
              <Stepper
                value={current}
                min={min}
                max={max}
                disabled={!sm.canSelfServe}
                busy={busyId === seatKey(c)}
                onChange={next => {
                  if (next < current && !sm.allowDowngrade) {
                    toast.error('Contact your account team to reduce your commitment')
                    return
                  }
                  setConfirm({ c, target: next })
                }}
              />
            ) : (
              <Mono style={{ fontSize: 13.5, fontWeight: 600 }}>{current}</Mono>
            )}
          </div>
        )
      })}
      {!sm.canSelfServe && (
        <ManagedNote>
          Managed by your account team — contact them to change seats or commitments.
        </ManagedNote>
      )}
      {levelOf && (
        <LevelChangeModal
          component={levelOf}
          subscriptionId={sm.subscription?.id ?? ''}
          currency={currency}
          periodEnd={sm.subscription?.currentPeriodEnd}
          allowLower={sm.levelDowngrade}
          busy={busyId === levelOf.componentId}
          onClose={() => setLevelOf(null)}
          onConfirm={async level => {
            setBusyId(levelOf.componentId)
            try {
              const res = await sm.setLevel(levelOf.componentId, level.priceId)
              toast.success(
                res.status === AdjustmentStatus.ADJUSTMENT_SCHEDULED
                  ? res.effectiveDate
                    ? `Level changes on ${date(res.effectiveDate)}`
                    : 'Level change scheduled for period end'
                  : appliedMessage('Level changed', res.invoicePending)
              )
              setLevelOf(null)
            } catch (e) {
              toast.error(e instanceof Error ? e.message : 'Unable to change level')
            } finally {
              setBusyId(null)
            }
          }}
        />
      )}
      <QuantityChangeModal
        change={
          confirm
            ? {
                name: confirm.c.name,
                fee: confirm.c.unitFee,
                current: Number(confirm.c.current),
                target: confirm.target,
                subscriptionId: sm.subscription?.id ?? '',
                kind: 'seats',
                targetId: confirm.c.componentId,
                seatAddOnId: confirm.c.addOnId,
              }
            : null
        }
        currency={currency}
        periodEnd={sm.subscription?.currentPeriodEnd}
        busy={!!confirm && busyId === seatKey(confirm.c)}
        onClose={() => setConfirm(null)}
        onConfirm={async () => {
          if (!confirm) return
          await setQty(confirm.c, confirm.target)
          setConfirm(null)
        }}
      />
    </PanelCard>
  )
}

/** How a usage-levels charge's sizes read: money when its metric counts the currency. */
const capacityUnit = (c: AdjustableComponent, currency: string): LevelUnit => ({
  money: unitIsCurrency({ plural: c.unit ?? null }, currency),
  plural: c.unit,
})

const levelOfFee = (level: CapacityLevel) => ({
  included: level.included,
  rate: level.fee?.amount ?? '0',
})

/**
 * Moves a usage-levels charge to another level of the plan. The amounts are the server's preview,
 * billed by the same path that applies the change: a higher level is billed now, prorated for the
 * days left; a lower one takes effect at the end of the period, nothing charged now.
 */
const LevelChangeModal = ({
  component,
  subscriptionId,
  currency,
  periodEnd,
  allowLower,
  busy,
  onClose,
  onConfirm,
}: {
  component: AdjustableComponent
  subscriptionId: string
  currency: string
  periodEnd?: string
  allowLower: boolean
  busy: boolean
  onClose: () => void
  onConfirm: (level: CapacityLevel) => void
}) => {
  const [picked, setPicked] = useState<CapacityLevel | null>(null)
  const unit = capacityUnit(component, currency)
  const held = BigInt(component.current)
  const period = component.unitFee?.cadence ?? SubscriptionFeeBillingPeriod.MONTHLY
  const preview = useQuery(
    previewLevelChange,
    { subscriptionId, componentId: component.componentId, priceId: picked?.priceId ?? '' },
    { enabled: !!picked }
  )
  const p = preview.data?.preview
  const raises = !!picked && picked.included > held
  const immediate = p?.immediate ?? raises

  return (
    <Modal open onClose={onClose} maxWidth={460}>
      <div style={{ padding: 24, display: 'flex', flexDirection: 'column', gap: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 16, fontWeight: 600 }}>Change {component.name}</span>
          <ModalCloseButton onClose={onClose} />
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {component.levels.map(level => {
            const lower = level.included < held
            const unavailable = level.current || (lower && !allowLower)
            const selected = picked?.priceId === level.priceId
            return (
              <button
                key={level.priceId}
                type="button"
                disabled={unavailable}
                onClick={() => setPicked(level)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 10,
                  padding: '11px 14px',
                  borderRadius: 'var(--mtp-r-ctrl)',
                  border: selected
                    ? '1px solid var(--mtp-accent-ink)'
                    : '1px solid var(--mtp-border)',
                  background: selected ? 'var(--mtp-accent-weak)' : 'var(--mtp-surface)',
                  color: 'var(--mtp-text)',
                  fontFamily: 'inherit',
                  fontSize: 13,
                  textAlign: 'left',
                  cursor: unavailable ? 'default' : 'pointer',
                  opacity: unavailable && !level.current ? 0.55 : 1,
                }}
              >
                <span>{levelWords(levelOfFee(level), currency, period, unit)}</span>
                {level.current && <Pill badge={{ label: 'Current', tone: 'ok' }} />}
              </button>
            )
          })}
          {!allowLower && component.levels.some(l => l.included < held) && (
            <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
              Lower levels are arranged with your account team.
            </span>
          )}
        </div>

        {picked && (
          <div
            style={{
              border: '1px solid var(--mtp-border)',
              borderRadius: 'var(--mtp-r-ctrl)',
              padding: '14px 16px',
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--mtp-surface-2)',
            }}
          >
            <SummaryLine
              label="Level"
              value={`${levelSize(held, currency, unit)} → ${levelSize(picked.included, currency, unit)}`}
            />
            {immediate ? (
              <SummaryLine
                label="Due today"
                value={
                  preview.isLoading ? 'Calculating…' : p ? money(p.immediateTotalCents, currency) : '—'
                }
                accent
              />
            ) : (
              <SummaryLine
                label="Takes effect"
                value={date(p?.effectiveDate || periodEnd || '') || 'At period end'}
              />
            )}
          </div>
        )}

        {picked && (
          <p style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', lineHeight: 1.5, margin: 0 }}>
            {immediate
              ? `Your new level starts today, billed now for the ${
                  p?.proration ? `${p.proration.daysRemaining} ` : ''
                }days left in your billing period. This period's usage counts against each level for the days you held it.`
              : `Your level changes at the end of your current billing period${
                  periodEnd ? ` (${date(periodEnd)})` : ''
                }. You keep your current level until then; nothing is charged now.`}
          </p>
        )}
        {preview.isError && (
          <p style={{ fontSize: 12.5, color: 'var(--mtp-danger)', margin: 0 }}>
            {preview.error instanceof Error ? preview.error.message : 'Unable to preview this change'}
          </p>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 9 }}>
          <PButton variant="secondary" onClick={onClose}>
            Cancel
          </PButton>
          <PButton
            loading={busy}
            disabled={!picked || preview.isLoading || preview.isError}
            onClick={() => picked && onConfirm(picked)}
          >
            {!immediate
              ? 'Schedule change'
              : p && p.immediateTotalCents > 0n
                ? 'Confirm & pay'
                : 'Confirm change'}
          </PButton>
        </div>
      </div>
    </Modal>
  )
}

/** One upcoming-invoice line, with an expandable daily-usage chart for metered charges. */
const PreviewLineRow = ({
  item,
  currency,
  subscriptionId,
  last,
}: {
  item: LineItem
  currency: string
  subscriptionId: string
  last: boolean
}) => {
  const [showUsage, setShowUsage] = useState(false)
  const metered = !!item.metricId
  return (
    <div
      style={{
        padding: '11px 0',
        borderBottom: last ? 'none' : '1px solid var(--mtp-border)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
        <span style={{ fontSize: 13, color: 'var(--mtp-text)' }}>{item.name}</span>
        <Mono style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', whiteSpace: 'nowrap' }}>
          {money(item.subtotal, currency)}
        </Mono>
      </div>
      {metered && (
        <div style={{ marginTop: 8 }}>
          <button
            type="button"
            onClick={() => setShowUsage(s => !s)}
            className="mtp-link"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              fontSize: 12,
              fontWeight: 500,
              color: 'var(--mtp-text-2)',
              background: 'none',
              border: 'none',
              padding: 0,
              cursor: 'pointer',
            }}
          >
            <ChevronDown
              size={13}
              style={{ transition: 'transform 0.15s', transform: showUsage ? 'none' : 'rotate(-90deg)' }}
            />
            {showUsage ? 'Hide daily usage' : 'Show daily usage'}
          </button>
          {showUsage && (
            <div style={{ marginTop: 10 }}>
              <UsageChart
                subscriptionId={subscriptionId}
                metricId={item.metricId!}
                startDate={item.startDate || undefined}
                endDate={item.endDate || undefined}
                groupByDimensions={
                  Object.keys(item.groupByDimensions).length > 0 ? item.groupByDimensions : undefined
                }
              />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

const PreviewTab = ({
  sm,
  currency,
  subscriptionId,
}: {
  sm: ReturnType<typeof useSubscription>
  currency: string
  subscriptionId: string
}) => {
  const inv = sm.upcomingInvoice
  if (sm.isUpcomingLoading)
    return (
      <div style={{ padding: 32, display: 'flex', justifyContent: 'center' }}>
        <Spinner size={18} />
      </div>
    )
  if (sm.isUpcomingError)
    return (
      <CenterState
        title="Preview unavailable"
        hint="This cycle's charges can't be computed right now. Try again in a few minutes."
      />
    )
  if (!inv || inv.lineItems.length === 0)
    return (
      <CenterState
        title="Nothing to preview"
        hint="Charges for the current cycle will appear here as they accrue."
      />
    )
  return (
    <PanelCard
      title="This cycle"
      action={
        <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
          {date(inv.periodStart)} – {date(inv.periodEnd)}
        </span>
      }
    >
      <div style={{ padding: '4px 20px 8px' }}>
        {inv.lineItems.map((li, i) => (
          <PreviewLineRow
            key={li.id || i}
            item={li}
            currency={currency}
            subscriptionId={subscriptionId}
            last={i === inv.lineItems.length - 1}
          />
        ))}
      </div>
      <Divider />
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          padding: '13px 20px',
          background: 'var(--mtp-surface-2)',
        }}
      >
        <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>Estimated total</span>
        <Mono style={{ fontSize: 15, fontWeight: 600 }}>{money(inv.total, inv.currency)}</Mono>
      </div>
    </PanelCard>
  )
}

interface PendingQtyChange {
  name: string
  fee?: ComponentFee
  current: number
  target: number
  subscriptionId: string
  // Identifies which server preview/apply path this change uses.
  kind: 'addon' | 'seats'
  // add_on_id for 'addon', price component_id for 'seats'.
  targetId: string
  // 'seats' charged by an add-on: its add_on_id, `targetId` then empty.
  seatAddOnId?: string
}

/** A seat charge's key: its plan charge, else the add-on charging it. */
const seatKey = (c: AdjustableComponent) => c.componentId || c.addOnId || ''

// Fees that bill a fixed amount per unit each period — the only ones we can give
// a reliable recurring estimate for. Usage/capacity depend on metered consumption.
const RECURRING_FEES = new Set<FeeType>([FeeType.RATE, FeeType.SLOT, FeeType.EXTRA_RECURRING])

const SummaryLine = ({
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

/**
 * Confirms a stepper-driven quantity change (add-on or seat/commitment) before
 * it is applied. The "due now" amount is the server preview — computed by the
 * same proration/invoicing path that applies the change, so it matches the
 * eventual charge exactly. Increases bill immediately (prorated); reductions are
 * deferred to the end of the current period (no charge now).
 */
const QuantityChangeModal = ({
  change,
  currency,
  periodEnd,
  busy,
  onClose,
  onConfirm,
}: {
  change: PendingQtyChange | null
  currency: string
  periodEnd?: string
  busy: boolean
  onClose: () => void
  onConfirm: () => void
}) => {
  const delta = change ? change.target - change.current : 0
  const isIncrease = delta > 0
  const isRemoval = !!change && change.target === 0 && change.current > 0
  const fee = change?.fee
  const unitCents = fee != null ? Math.round(Number(fee.amount) * 100) : NaN
  const hasUnitCents = Number.isFinite(unitCents) && unitCents > 0
  const recurringDelta =
    fee && hasUnitCents && RECURRING_FEES.has(fee.feeType) ? unitCents * delta : null
  const signed = (cents: number) => `${cents >= 0 ? '+' : '−'}${money(Math.abs(cents), currency)}`

  // Authoritative cost preview from the backend (same path that bills the change).
  // Two hooks, only the relevant one enabled — connect-query needs a stable method.
  const isAddon = change?.kind === 'addon'
  const addonPreview = useQuery(
    previewAddOnQuantity,
    {
      subscriptionId: change?.subscriptionId ?? '',
      addOnId: change?.targetId ?? '',
      quantity: change?.target ?? 0,
    },
    { enabled: !!change && isAddon }
  )
  const seatsPreview = useQuery(
    previewSeats,
    {
      subscriptionId: change?.subscriptionId ?? '',
      componentId: change?.targetId ?? '',
      quantity: change?.target ?? 0,
      addOnId: change?.seatAddOnId || undefined,
    },
    { enabled: !!change && !isAddon }
  )
  const pq = isAddon ? addonPreview : seatsPreview
  const preview = pq.data?.preview
  const previewLoading = !!change && pq.isLoading

  // Prefer the server's verdict on whether this bills now; fall back to the sign
  // of the delta until the preview lands.
  const immediate = preview?.immediate ?? isIncrease
  const proration = preview?.proration

  return (
    <Modal open={!!change} onClose={onClose} maxWidth={440}>
      {change && (
        <div style={{ padding: 24, display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <span style={{ fontSize: 16, fontWeight: 600 }}>
              {isRemoval ? `Remove ${change.name}?` : `Update ${change.name}`}
            </span>
            <ModalCloseButton onClose={onClose} />
          </div>

          <div
            style={{
              border: '1px solid var(--mtp-border)',
              borderRadius: 'var(--mtp-r-ctrl)',
              padding: '14px 16px',
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--mtp-surface-2)',
            }}
          >
            <SummaryLine label="Quantity" value={`${change.current} → ${change.target}`} />
            <SummaryLine label="Unit price" value={feeLabel(change.fee, currency)} />
            {recurringDelta != null && recurringDelta !== 0 && (
              <SummaryLine
                label="Ongoing"
                value={`${signed(recurringDelta)} ${periodSuffix(fee!.cadence)}`.trim()}
              />
            )}
            {immediate && (
              <SummaryLine
                label="Due today"
                value={
                  previewLoading
                    ? 'Calculating…'
                    : preview
                      ? money(preview.immediateTotalCents, currency)
                      : '—'
                }
                accent
              />
            )}
          </div>

          <p style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', lineHeight: 1.5, margin: 0 }}>
            {immediate
              ? proration
                ? `Billed now, prorated for the ${proration.daysRemaining} day${
                    proration.daysRemaining === 1 ? '' : 's'
                  } remaining in your current billing period.`
                : 'This increase is billed now, prorated for the days remaining in your current billing period.'
              : `This reduction takes effect at the end of your current billing period${
                  periodEnd ? ` (${date(periodEnd)})` : ''
                }. You keep what you have until then.`}
          </p>

          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 9 }}>
            <PButton variant="secondary" onClick={onClose}>
              Cancel
            </PButton>
            <PButton
              variant={isRemoval ? 'danger' : 'primary'}
              loading={busy}
              disabled={previewLoading}
              onClick={onConfirm}
            >
              {isRemoval
                ? 'Remove'
                : !immediate
                  ? 'Schedule change'
                  : preview && preview.immediateTotalCents > 0n
                    ? 'Confirm & pay'
                    : 'Confirm change'}
            </PButton>
          </div>
        </div>
      )}
    </Modal>
  )
}

