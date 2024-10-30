import {
  Badge,
  Button,
  ScrollArea,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import {
  ActivityIcon,
  ArmchairIcon,
  ArrowDownIcon,
  CircleDollarSignIcon,
  Clock4Icon,
  ParkingMeterIcon,
  UngroupIcon,
} from 'lucide-react'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { useAddComponent } from '@/features/billing/plans/pricecomponents/utils'
import { PriceComponentType } from '@/features/billing/plans/types'
export const PriceComponentPanel: React.FC = () => {
  const navigate = useNavigate()
  return (
    <Sheet open={true} onOpenChange={() => navigate('..')}>
      <SheetContent size="medium">
        <SheetHeader className="border-b border-border pb-3 mb-3">
          <SheetTitle>Pick a pricing model</SheetTitle>
          <SheetDescription>Add a new price component to your plan</SheetDescription>
        </SheetHeader>
        <div>
          <ScrollArea className="max-h-[calc(100vh-130px)] h-full px-2">
            <div className="p-6 pt-0 text-sm text-foreground">
              <Button disabled variant="secondary">
                Import from another plan (soon)
              </Button>
            </div>

            <div className="pl-4 pb-2  text-sm font-semibold text-muted-foreground">
              Standard models
            </div>
            <Row
              to="rate"
              label="Subscription Rate"
              description="Standard subscription price with a fixed rate per billing period. You can define different rates per committed period."
              icon={<UngroupIcon size={14} />}
            />
            <Row
              to="slot"
              label="Slot-based"
              description="Perfect for Seats, Licenses or other purchasable entities. Price is based on the number of active slots. You can define different rates per committed period."
              icon={<ArmchairIcon size={14} />}
            />
            <Row
              to="capacity"
              label="Capacity commitment"
              description="Threshold-based pricing system where users pay based on the capacity they purchase, with overage charges for additional usage."
              icon={<ParkingMeterIcon size={14} />}
            />
            <div className="pl-4 pt-6 pb-2  text-sm font-semibold text-muted-foreground">
              Pay-as-you-go models
            </div>

            <Row
              to="usage"
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

            <div className="pl-4 pt-6 pb-2 text-sm font-semibold text-muted-foreground">
              Additional charges
            </div>
            <Row
              to="oneTime"
              label="One-time charge"
              description="Charge your customers once when their subscription starts. Ideal for an implementation fee."
              icon={<ArrowDownIcon size={14} />}
            />
            <Row
              to="extraRecurring"
              label="Recurring charge"
              description="A recurring fee outside of the standard subscription rate or period, ex: a quarterly maintenance fee"
              icon={<Clock4Icon size={14} />}
            />
          </ScrollArea>
        </div>
      </SheetContent>
    </Sheet>
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
  const base = 'flex items-center p-3 m-2  border rounded-md border-border '

  const standardClassName = `${base} cursor-pointer group bg-card hover:bg-accent`
  const disabledClassName = `${base} cursor-default bg-secondary text-muted-foreground`

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
      <div className="flex items-center justify-center w-6 h-6  rounded-full mr-4 group-hover:text-brand group-hover:bg-transparent">
        {icon}
      </div>
      <div className="flex flex-col w-full">
        <div className="flex flex-row w-full">
          <span className="mb-2 text-sm font-semibold flex-grow items-center">{label}</span>
          {disabled && (
            <>
              <Badge variant="secondary" className="text-xs pr-1">
                soon
              </Badge>
              {/* <span className="text-xs  flex-row items-center flex cursor-pointer hover:text-green-900">
                <ArrowBigUp size={14} /> (5)
              </span> */}
            </>
          )}
        </div>

        <span className="text-sm text-muted-foreground">{description}</span>
      </div>
    </div>
  )
}

// TODO edit/create => just switch the PriceCOmponent panel, do not use a sider after selecting the feeType
