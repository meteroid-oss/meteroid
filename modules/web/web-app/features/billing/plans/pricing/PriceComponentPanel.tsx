import { Badge, SidePanel } from '@ui/components'
import {
  ActivityIcon,
  ArmchairIcon,
  ArrowDownIcon,
  Clock4Icon,
  ParkingMeterIcon,
  UngroupIcon,
  CircleDollarSignIcon,
  ArrowBigUp,
} from 'lucide-react'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { useAddComponent } from '@/features/billing/plans/pricecomponents/utils'
import { PriceComponentType } from '@/features/billing/plans/types'

export const PriceComponentPanel: React.FC = () => {
  const navigate = useNavigate()
  return (
    <SidePanel
      visible
      hideFooter
      header={<SidePanel.HeaderTitle>Pick a pricing model</SidePanel.HeaderTitle>}
      onCancel={() => navigate('..')}
    >
      <div className="px-2">
        <div className="p-6 pt-0 text-sm text-muted-foreground">
          Import from another plan (soon)
        </div>

        <div className="pl-4 pb-2  text-sm font-semibold text-slate-1000">Standard models</div>
        <Row
          to="rate"
          label="Subscription Rate"
          description="Standard subscription price with a fixed rate per billing period. You can define different rates per committed period."
          icon={<UngroupIcon size={14} />}
        />
        <Row
          to="slot_based"
          label="Slot-based"
          description="Perfect for Seats, Licenses or other purchasable entities. Price is based on the number of active slots. You can define different rates per committed period."
          icon={<ArmchairIcon size={14} />}
        />
        <Row
          to="capacity"
          label="Capacity commitment"
          description="Threshold-based pricing system where users pay based on the capacity they purchase, with overage charges for additional usage or upsell opportunities."
          icon={<ParkingMeterIcon size={14} />}
        />
        <div className="pl-4 pt-6 pb-2  text-sm font-semibold text-slate-1000">
          Pay-as-you-go models
        </div>

        <Row
          to="usage_based"
          label="Usage-based"
          description="Charge your customers based on their usage of your product during the last billing period."
          icon={<ActivityIcon size={14} />}
        />
        <Row
          label="Basis points"
          description="Designed for fintechs, charge your customers instantly a percentage for each transactional event."
          icon={<CircleDollarSignIcon size={14} />}
          disabled
        />

        <div className="pl-4 pt-6 pb-2 text-sm font-semibold text-slate-1000">
          Additional charges
        </div>
        <Row
          to="one_time"
          label="One-time charge"
          description="Charge your customers once when their subscription starts. Ideal for an implementation fee."
          icon={<ArrowDownIcon size={14} />}
        />
        <Row
          to="recurring"
          label="Recurring charge"
          description="A recurring fee outside of the standard subscription rate or period, ex: a quarterly maintenance fee"
          icon={<Clock4Icon size={14} />}
        />
      </div>
    </SidePanel>
  )
}

type RowProps = {
  to?: PriceComponentType
  label: ReactNode
  description: ReactNode
  icon: ReactNode
  disabled?: boolean
}

const Row: React.FC<RowProps> = ({ to, label, description, icon, disabled = false }) => {
  const base = 'flex items-center p-2 m-2  border rounded-md border-slate-400 '

  const standardClassName = `${base} cursor-pointer group hover:bg-gray-100 hover:border-brand-1000`
  const disabledClassName = `${base} cursor-default bg-gray-200`

  const addComponent = useAddComponent()
  const navigate = useNavigate()

  const onClick = () => {
    if (!to) return
    addComponent(to)
    navigate('..')
  }

  return (
    <div
      onClick={disabled ? undefined : onClick}
      className={disabled ? disabledClassName : standardClassName}
    >
      <div className="flex items-center justify-center w-6 h-6 bg-gray-200 rounded-full mr-4 group-hover:text-brand-1000 group-hover:bg-transparent">
        {icon}
      </div>
      <div className="flex flex-col w-full">
        <div className="flex flex-row w-full">
          <span className="mb-2 text-sm font-semibold flex-grow items-center">{label}</span>
          {disabled && (
            <>
              <Badge variant="secondary" className="text-xs pr-1">
                Soon
              </Badge>
              <span className="text-xs  flex-row items-center flex cursor-pointer hover:text-green-900">
                <ArrowBigUp size={14} /> (5)
              </span>
            </>
          )}
        </div>

        <span className="text-sm text-slate-1000">{description}</span>
      </div>
    </div>
  )
}

// TODO edit/create => just switch the PriceCOmponent panel, do not use a sider after selecting the feeType
