import { A, D, F, pipe } from '@mobily/ts-belt'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
} from '@ui/components'
import { cn } from '@ui/lib'
import { Plus } from 'lucide-react'

import { useQuery } from '@/lib/connectrpc'
import { PlanOverview, PlanStatus, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

interface Props {
  value?: PlanVersion['id']
  onChange: (id: PlanVersion['id']) => void
  /** Classes for the trigger, e.g. to fill its field instead of the standalone 180px. */
  triggerClassName?: string
  placeholder?: string
  /** Where a new plan is made: offered last in the list, apart. It opens in a new tab, as a
   *  new plan is a draft until published, and the form here keeps its state meanwhile. */
  createPlanHref?: string
}

// Radix Select only moves between items: the "create" entry is one, with a value never stored.
const CREATE_PLAN = '__create_plan__'

export const SubscribablePlanVersionSelect = ({
  value,
  onChange,
  triggerClassName,
  placeholder = 'Choose one',
  createPlanHref,
}: Props) => {
  const plansQuery = useQuery(listPlans, {
    filters: {
      statuses: [PlanStatus.ACTIVE],
      types: [],
    },
  })

  const plansByFamily = pipe(
    plansQuery.data?.plans,
    F.defaultTo([] as PlanOverview[]),
    A.filter(p => !!p.activeVersion?.id),
    A.groupBy(p => p.productFamilyName)
  )

  const selectedPlan = plansQuery.data?.plans.find(p => p.activeVersion?.id === value)?.name

  return (
    <Select
      value={value}
      onValueChange={next => {
        if (next === CREATE_PLAN) {
          if (createPlanHref) window.open(createPlanHref, '_blank', 'noopener')
          return
        }
        onChange(next)
      }}
    >
      <SelectTrigger className={cn('w-[180px]', triggerClassName)}>
        {selectedPlan ?? placeholder}
      </SelectTrigger>
      <SelectContent>
        {pipe(
          plansByFamily,
          D.toPairs,
          A.map(([family, plans]) => (
            <SelectGroup key={family}>
              {/* <SelectLabel className="SelectLabel">{family}</SelectLabel> */}
              {plans?.map(p => (
                <SelectItem key={p.activeVersion?.id} value={p.activeVersion?.id ?? 'none'}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectGroup>
          ))
        )}
        {createPlanHref && (
          <>
            <SelectSeparator />
            <SelectItem value={CREATE_PLAN}>
              <span className="flex items-center gap-2">
                <Plus className="h-3.5 w-3.5" aria-hidden />
                Create new plan
              </span>
            </SelectItem>
          </>
        )}
      </SelectContent>
    </Select>
  )
}
