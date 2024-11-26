import { A, D, F, pipe } from '@mobily/ts-belt'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
} from '@ui/components'

import { useQuery } from '@/lib/connectrpc'
import { PlanOverview, PlanStatus, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

interface Props {
  value?: PlanVersion['id']
  onChange: (id: PlanVersion['id']) => void
}

export const SubscribablePlanVersionSelect = ({ value, onChange }: Props) => {
  const plansQuery = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.DATE_DESC,
    filters: {
      statuses: [PlanStatus.ACTIVE],
      types: [],
    },
  })

  const plansByFamily = pipe(
    plansQuery.data?.plans,
    F.defaultTo([] as PlanOverview[]),
    A.groupBy(p => p.productFamilyName)
  )

  const selectedPlan = plansQuery.data?.plans.find(p => p.id === value)?.name

  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="w-[180px]">{selectedPlan ?? 'Choose one'}</SelectTrigger>
      <SelectContent>
        {pipe(
          plansByFamily,
          D.toPairs,
          A.map(([family, plans]) => (
            <SelectGroup key={family}>
              <SelectLabel className="SelectLabel">{family}</SelectLabel>
              {plans?.map(p => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectGroup>
          ))
        )}
      </SelectContent>
    </Select>
  )
}
