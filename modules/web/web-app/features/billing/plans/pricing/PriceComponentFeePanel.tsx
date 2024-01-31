import { SidePanel } from '@ui/components'
import React, { ReactNode } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { match } from 'ts-pattern'
import { z } from 'zod'

import { PriceComponentType } from '@/features/billing/plans/types'
import { useZodForm } from '@/hooks/useZodForm'
import { useTypedParams } from '@/utils/params'

const schema = z.object({
  name: z.string().min(1),
  feeType: z.enum(['rate', 'slotBased', 'capacity', 'usageBased', 'scheduled', 'oneTime']),
  productItemId: z.string().uuid().optional(),
})
export const PriceComponentFeePanel: React.FC = () => {
  const methods = useZodForm({
    schema: schema,
    defaultValues: {},
  })
  const navigate = useNavigate()

  const { feeType } = useTypedParams<{ feeType: PriceComponentType }>()

  console.log(z)

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

type RowProps = {
  to: PriceComponentType
  label: ReactNode
  description: ReactNode
  icon: ReactNode
}

const Row: React.FC<RowProps> = ({ to, label, description, icon }) => (
  <Link
    to={to}
    className="flex items-center p-2 m-2 cursor-pointer hover:bg-gray-100 border rounded-md border-slate-400 hover:border-brand-1000 group"
  >
    <div className="flex items-center justify-center w-6 h-6 bg-gray-200 rounded-full mr-4 group-hover:text-brand-1000 group-hover:bg-transparent">
      {icon}
    </div>
    <div className="flex flex-col w-full">
      <span className="mb-2 text-sm font-semibold ">{label}</span>
      <span className="text-sm text-scale-1000">{description}</span>
    </div>
  </Link>
)
