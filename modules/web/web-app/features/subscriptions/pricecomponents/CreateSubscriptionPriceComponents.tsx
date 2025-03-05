import { useQuery } from '@/lib/connectrpc'
import { BillingPeriod } from '@/lib/mapping'
import { mapFeeType } from '@/lib/mapping/feesFromGrpc'
import { PriceComponent } from '@/lib/schemas/plans'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from '@ui/components'

import { Activity, Calendar, Edit2, Package, Trash2, Users } from 'lucide-react'
import { match } from 'ts-pattern'

export const CreateSubscriptionPriceComponents = ({
  planVersionId,
}: {
  planVersionId: PlanVersion['id']
}) => {
  const planPriceComponents = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
  )?.data?.components.map(
    c =>
      ({
        id: c.id,
        name: c.name,
        localId: c.localId,
        fee: c.fee ? mapFeeType(c.fee.feeType) : undefined,
        productId: c.productId,
      }) as PriceComponent
  )

  const subscriptionPriceComponents: ComponentState[] = (planPriceComponents ?? []).map(c => ({
    originalComponent: c,
    state: 'default',
    feeType: c.fee.fee,
    id: c.id,
    name: c.name,
  }))

  return (
    <div className="grid gap-y-4">
      {subscriptionPriceComponents?.map(priceComponent => (
        <PriceComponentCard component={priceComponent} key={priceComponent.id} />
      ))}
      {!subscriptionPriceComponents?.length && <span>No price components</span>}
    </div>
  )
}

// UI-friendly component state
type ComponentState = {
  id?: string
  name: string
  feeType: 'rate' | 'slot' | 'capacity' | 'usage' | 'extraRecurring' | 'oneTime'
  originalComponent?: PriceComponent // Keep reference to original
  state: 'default' | 'extra' | 'parameterized' | 'overridden' | 'removed'
  // Unified configuration that covers all possible states
  parameters?: {
    initialSlotCount?: number
    billingPeriod?: BillingPeriod
    committedCapacity?: number
  }
  // override?: SubscriptionFee
  // config: {} / adjustments
}

const ComponentConfig = ({ originalComponent }: { originalComponent: PriceComponent }) => {
  if (
    (originalComponent.fee.fee === 'rate' || originalComponent.fee.fee === 'slot') &&
    originalComponent.fee.data.rates.length > 1
  ) {
    return (
      <>
        <Label>Billing period</Label>
        <Select>
          {' '}
          {/*  value={value} onValueChange={onValueChange} */}
          <SelectTrigger className="w-[180px]">{'Choose one'}</SelectTrigger>
          <SelectContent>
            {originalComponent.fee.data.rates.map(v => (
              <>
                <SelectItem key={v.term} value={v.term}>
                  {v.term.toLowerCase()}
                  {/* ({v.price}/{v.term.toLowerCase().slice(0, 2)}) •{' '}
                  {v.price * (v.term === 'ANNUAL' ? 1 : 12)}/year */}
                </SelectItem>
              </>
            ))}
          </SelectContent>
        </Select>
      </>
    )
  }

  if (originalComponent.fee.fee === 'capacity') {
    return (
      <>
        <Label>Committed capacity</Label>
        <Select>
          <SelectTrigger className="w-[180px]">{'Choose one'}</SelectTrigger>
          <SelectContent>
            {originalComponent.fee.data.thresholds.map(v => (
              <>
                <SelectItem key={v.includedAmount} value={v.includedAmount}>
                  {v.includedAmount.toLowerCase()} included
                </SelectItem>
              </>
            ))}
          </SelectContent>
        </Select>
      </>
    )
  }

  return null
}

const PriceComponentCard = ({ component }: { component: ComponentState }) => {
  const getFeeTypeIcon = (fee: ComponentState['feeType']) => {
    if (fee === 'rate') return <Calendar className="h-5 w-5 " />
    if (fee === 'usage') return <Activity className="h-5 w-5 " />
    if (fee === 'slot') return <Users className="h-5 w-5 " />
    return <Package className="h-5 w-5 " />
  }

  const getFeeTypeText = (fee: ComponentState['feeType']) => {
    return match(fee)
      .with('rate', () => 'Fixed rate pricing')
      .with('usage', () => 'Usage-based pricing')
      .with('slot', () => 'Per-unit slot pricing')
      .with('capacity', () => 'Capacity-based pricing')
      .with('oneTime', () => 'One-time pricing')
      .with('extraRecurring', () => 'Extra recurring pricing')
      .exhaustive()
  }

  return (
    <>
      <Card key={component.id}>
        <CardHeader className="flex flex-row items-start justify-between pb-2">
          <div className="flex gap-3">
            {getFeeTypeIcon(component.feeType)}
            <div>
              <CardTitle className="text-lg">{component.name}</CardTitle>
              <div className="text-sm text-gray-500 mt-1">{getFeeTypeText(component.feeType)}</div>
            </div>
          </div>
          <div className="flex gap-2">
            <Button variant="ghost" size="icon" disabled>
              <Edit2 className="h-4 w-4 " />
            </Button>
            <Button variant="destructiveGhost" size="icon" disabled>
              <Trash2 className="h-4 w-4 " />
            </Button>
          </div>
        </CardHeader>
        <CardContent className="pt-4">
          {component.originalComponent && (
            <ComponentConfig originalComponent={component.originalComponent} />
          )}
          {/* {renderComponentConfig(component)}
          {calculateAnnualEstimate(component) !== null && (
            <div className="mt-4 text-sm text-gray-600">
              Estimated annual cost: {formatPrice(calculateAnnualEstimate(component))}
            </div>
          )} */}
        </CardContent>
      </Card>
    </>
  )
}
