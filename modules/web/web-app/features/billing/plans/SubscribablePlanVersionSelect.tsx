import { A, D, F, pipe } from '@mobily/ts-belt'
import {
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  Select,
  SelectTrigger,
} from '@ui/components'

import { useQuery } from '@/lib/connectrpc'
import { ListSubscribablePlanVersion, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listSubscribablePlanVersion } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

interface Props {
  value?: PlanVersion['id']
  onChange: (id: PlanVersion['id']) => void
}

export const SubscribablePlanVersionSelect = ({ value, onChange }: Props) => {
  const getPlanQuery = useQuery(listSubscribablePlanVersion)

  const plansByFamily = pipe(
    getPlanQuery.data?.planVersions,
    F.defaultTo([] as ListSubscribablePlanVersion[]),
    A.groupBy(p => p.productFamilyName)
  )

  const selectedPlan = getPlanQuery.data?.planVersions.find(p => p.id === value)?.planName

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
                  {p.planName}
                </SelectItem>
              ))}
            </SelectGroup>
          ))
        )}
      </SelectContent>
    </Select>
  )
}
