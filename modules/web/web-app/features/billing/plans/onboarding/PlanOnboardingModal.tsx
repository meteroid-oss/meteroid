import { Button, Modal } from '@ui2/components'
import { FC } from 'react'
import { useNavigate } from 'react-router-dom'

import { useAddComponent } from '@/features/billing/plans/pricecomponents/utils'
import CapacityPricingModelSVG from '@/features/billing/plans/svg/capacity-pricing-model.svg?react'
import FixedPricingModelSVG from '@/features/billing/plans/svg/fixed-pricing-model.svg?react'
import PayAsYouGoPricingModelSVG from '@/features/billing/plans/svg/pay-as-you-go-pricing-model.svg?react'
import SlotsPricingModelSVG from '@/features/billing/plans/svg/slots-pricing-model.svg?react'
import { PriceComponentType } from '@/features/billing/plans/types'

export const PlanOnboardingModal: FC = () => {
  const onSelectCancel = () => {
    navigate('..')
  }

  const addComponent = useAddComponent()

  const navigate = useNavigate()

  const startWithPricingModel = (model: PriceComponentType) => {
    addComponent(model)
    navigate('..')
  }

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={<h2 className="font-semibold">Quick start</h2>}
      size="xxlarge"
      onCancel={onSelectCancel}
      customFooter={
        <Button variant="ghost" onClick={onSelectCancel}>
          Skip to plan details
        </Button>
      }
    >
      <div className="px-4 py-3 h-full flex flex-row">
        <div className="w-2/3 text-center p-4">
          <div>
            <h2 className="text-lg font-semibold mb-4">Pick a base pricing model</h2>
            <div className="text-sm text-muted-foreground">
              You can add additional price components and addons later
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4 mt-4">
            <PricingModelCard
              title="Standard rate"
              subtitle="Standard subscription price with a fixed rate per billing period"
              hero={<FixedPricingModelSVG />}
              action={() => startWithPricingModel('rate')}
            />
            <PricingModelCard
              title="Slot-based price"
              subtitle="Perfect for Seats or Licenses. Price is based on a metered feature."
              hero={<SlotsPricingModelSVG />}
              action={() => startWithPricingModel('slot_based')}
            />
            {/* examples: loops.so, mailgun, ... */}
            <PricingModelCard
              title="Capacity scale"
              subtitle="Variable prices based on the committed usage"
              hero={<CapacityPricingModelSVG />}
              action={() => startWithPricingModel('capacity')}
            />
            <PricingModelCard
              title="Pay-as-you-go"
              subtitle="Flexible usage-based pricing with no single main fixed fee or commitment"
              hero={<PayAsYouGoPricingModelSVG />}
              action={() => startWithPricingModel('usage_based')}
            />
          </div>
        </div>
        <div className="flex flex-col items-center">
          <div className="flex-grow w-0.5 bg-border pt-2"></div>
          <div>or</div>
          <div className="flex-grow w-0.5 bg-border pb-2"></div>
        </div>
        <div className="w-1/3 text-center h-3/4">
          <h2 className="text-lg font-semibold mb-4">Start from a template</h2>
          <div>Coming soon</div>
        </div>
      </div>
    </Modal>
  )
}

interface PricingModelCardProps {
  title: string
  subtitle: string
  hero: React.ReactNode
  action: () => void
}
export const PricingModelCard = (props: PricingModelCardProps) => {
  return (
    <div
      className="p-4 bg-card text-card-foreground hover:bg-accent border rounded-md cursor-pointer"
      onClick={props.action}
    >
      <h2 className="text-lg font-semibold pb-2">{props.title}</h2>
      <div>{props.hero}</div>
      <div>
        <span className="text-sm">{props.subtitle}</span>
      </div>
    </div>
  )
}
