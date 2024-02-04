import { SidePanel } from '@ui/components'
import { match } from 'ts-pattern'

import { PriceComponentType } from '@/features/billing/plans/types'
import { useTypedParams } from '@/utils/params'

export const PriceComponentFeePanel: React.FC = () => {
  const { feeType } = useTypedParams<{ feeType: PriceComponentType }>()

  const feeForm = match(feeType)
    .with('rate' as const, () => <div>rate</div>)
    .with('slot_based' as const, () => <div>slot-based</div>)
    .with('capacity' as const, () => <div>capacity</div>)
    .with('usage_based' as const, () => <div>usage-based</div>)
    .with('recurring' as const, () => <div>scheduled</div>)
    .with('one_time' as const, () => <div>one-time</div>)
    .otherwise(() => <div>Unknown fee type. Please contact the support</div>)

  return (
    <SidePanel
      visible={false}
      hideFooter
      header={<SidePanel.HeaderTitle>Price component configuration</SidePanel.HeaderTitle>}
    >
      {feeForm}
    </SidePanel>
  )
}
