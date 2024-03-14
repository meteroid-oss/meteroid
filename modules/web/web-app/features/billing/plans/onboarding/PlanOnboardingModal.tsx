import { Button, Modal, cn } from '@md/ui'
import { FC, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { useAddComponent } from '@/features/billing/plans/pricecomponents/utils'
import CapacityPricingModelSVG from '@/features/billing/plans/svg/capacity-pricing-model.svg?react'
import FixedPricingModelSVG from '@/features/billing/plans/svg/fixed-pricing-model.svg?react'
import PayAsYouGoPricingModelSVG from '@/features/billing/plans/svg/pay-as-you-go-pricing-model.svg?react'
import SlotsPricingModelSVG from '@/features/billing/plans/svg/slots-pricing-model.svg?react'
import { PriceComponentType } from '@/features/billing/plans/types'
import {
  RateDesignCard,
  SlotsDesignCard,
  CapacityDesignCard,
  UsageBasedDesignCard,
} from '@/features/billing/plans/onboarding/PricingModelDesignCards'

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

  const [selected, setSelected] = useState<PriceComponentType | null>(null)

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={<h2 className="font-semibold">Quick start</h2>}
      size="xxlarge"
      onCancel={onSelectCancel}
      customFooter={
        <>
          <Button variant="ghost" onClick={onSelectCancel}>
            Skip to plan details
          </Button>
          <Button
            variant="primary"
            onClick={() => startWithPricingModel(selected ?? 'rate')}
            disabled={!selected}
          >
            Continue
          </Button>
        </>
      }
    >
      <div className="px-4 py-3 h-full flex flex-col lg:flex-row ">
        <div className="w-2/3 text-center p-4 mx-auto">
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
              hero={<RateDesignCard />}
              action={() => setSelected('rate')}
              selected={selected === 'rate'}
            />
            <PricingModelCard
              title="Slot-based price"
              subtitle="Perfect for Seats or Licenses. Price is based on a metered feature."
              hero={<SlotsDesignCard />}
              action={() => setSelected('slot_based')}
              selected={selected === 'slot_based'}
            />
            {/* examples: loops.so, mailgun, ... */}
            <PricingModelCard
              title="Capacity scale"
              subtitle="Variable prices based on the committed usage"
              hero={<CapacityDesignCard />}
              action={() => setSelected('capacity')}
              selected={selected === 'capacity'}
            />
            <PricingModelCard
              title="Pay-as-you-go"
              subtitle="Flexible usage-based pricing with no single main fixed fee or commitment"
              hero={<UsageBasedDesignCard />}
              action={() => setSelected('usage_based')}
              selected={selected === 'usage_based'}
            />
          </div>
        </div>
        <div className="flex flex-row lg:flex-col items-center">
          <div className="flex-grow  bg-border h-0.5 mx-2 lg:w-0.5 lg:pt-2"></div>
          <div>or</div>
          <div className="flex-grow  bg-border h-0.5 mx-2 lg:w-0.5 lg:pb-2"></div>
        </div>
        <div className="w-1/3 text-center h-3/4 mx-auto">
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
  selected?: boolean
}
export const PricingModelCard = (props: PricingModelCardProps) => {
  return (
    <div
      className={cn(
        'p-4 bg-card text-card-foreground hover:bg-accent border rounded-md cursor-pointer group',
        props.selected ? 'border-primary' : ''
      )}
      onClick={props.action}
      onKeyUp={key => key.key === 'Enter' && props.action()}
      tabIndex={0}
    >
      <h2 className="text-lg font-semibold pb-2">{props.title}</h2>
      <div className="hidden lg:block">{props.hero}</div>
      <div>
        <span className="text-sm">{props.subtitle}</span>
      </div>
    </div>
  )
}
