import { usePortalNav } from '../context'
import { feeLabel } from '../format'
import { planPriceLines, planPriceShort } from '../planPrice'
import { Card, Mono, PButton } from '../primitives'

import type { AvailablePlan } from '@/rpc/portal/subscription/v1/subscription_pb'

export const UPGRADE_SECTION_ID = 'mtp-upgrade'

const planBullets = (plan: AvailablePlan): string[] => [
  ...(plan.price ? planPriceLines(plan.price, undefined, { creditTerms: true }) : []),
  ...(plan.description ? [plan.description] : []),
]

const priceLabel = (plan: AvailablePlan, currency: string) => {
  if (!plan.price) return feeLabel(plan.headlineFee, currency)
  const label = planPriceShort(plan.price)
  // A plan named "Pay as you go" needs no "Pay as you go" price tag beside its name.
  return label.toLowerCase() === plan.planName.toLowerCase() ? '' : label
}

/** The plans the customer can switch to, each opening the change-plan flow. */
export const UpgradeSection = ({
  plans,
  subscriptionId,
  currency,
}: {
  plans: AvailablePlan[]
  subscriptionId: string
  currency: string
}) => {
  const { openChangePlan } = usePortalNav()
  if (plans.length === 0) return null
  return (
    <section id={UPGRADE_SECTION_ID} aria-labelledby={`${UPGRADE_SECTION_ID}-h`}>
      <Card style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <h2 id={`${UPGRADE_SECTION_ID}-h`} style={{ margin: 0, fontSize: 15, fontWeight: 600 }}>
          Upgrade your plan
        </h2>
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
            gap: 12,
          }}
        >
          {plans.map((plan, index) => {
            const bullets = planBullets(plan)
            return (
              <div
                key={plan.planVersionId}
                style={{
                  border: `1px solid ${index === 0 ? 'var(--mtp-accent)' : 'var(--mtp-border-2)'}`,
                  borderRadius: 'var(--mtp-r-card)',
                  padding: 18,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    flexWrap: 'wrap',
                    justifyContent: 'space-between',
                    alignItems: 'baseline',
                    gap: 8,
                  }}
                >
                  <span style={{ fontSize: 16, fontWeight: 600 }}>{plan.planName}</span>
                  <Mono style={{ fontWeight: 600 }}>
                    {priceLabel(plan, currency)}
                  </Mono>
                </div>
                {bullets.length > 0 && (
                  <ul
                    style={{
                      margin: 0,
                      paddingLeft: 18,
                      fontSize: 13,
                      color: 'var(--mtp-text-2)',
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 4,
                    }}
                  >
                    {bullets.map(line => (
                      <li key={line}>{line}</li>
                    ))}
                  </ul>
                )}
                <PButton
                  variant={index === 0 ? 'primary' : 'secondary'}
                  onClick={() => openChangePlan(subscriptionId)}
                  style={{ marginTop: 'auto' }}
                >
                  Switch to {plan.planName}
                </PButton>
              </div>
            )
          })}
        </div>
      </Card>
    </section>
  )
}
