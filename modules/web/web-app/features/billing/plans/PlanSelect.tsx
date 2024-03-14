import { A, D, F, pipe } from '@mobily/ts-belt'
import {
  Input,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  Select,
  SelectTrigger,
} from '@ui/components'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { Plan, ListPlan } from '@/rpc/api/plans/v1/models_pb'
import { getPlanByExternalId, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

interface Props {
  value?: Plan['externalId']
  onChange: (id: Plan['externalId']) => void
}

export const PlanSelect = ({ value, onChange }: Props) => {
  const [search, setSearch] = useState('')

  const getPlanQuery = useQuery(
    getPlanByExternalId,
    {
      externalId: value ?? '',
    },
    { enabled: Boolean(value) }
  )

  const onValueChange = (value: string) => {
    onChange(value)
    setSearch('')
  }

  const plan = getPlanQuery.data

  return (
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger className="w-[180px]">
        {plan?.planDetails?.plan?.name ?? 'Choose one'}
      </SelectTrigger>
      <SelectContent>
        <Input
          key="input"
          className="mb-2"
          placeholder="Search.."
          autoFocus
          value={search}
          onChange={event => setSearch(event.target.value)}
        />
        <PlanItems search={search} />
      </SelectContent>
    </Select>
  )
}

const PlanItems = ({ search }: { search: string }) => {
  const query = useQuery(listPlans, {
    pagination: {
      limit: 20,
      offset: 0,
    },
    search: search.length > 0 ? search : undefined,
    orderBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  // const plans = query.data?.plans || []
  const plansByFamily = pipe(
    query.data?.plans,
    F.defaultTo([] as ListPlan[]),
    A.groupBy(p => p.productFamilyName)
  )

  return (
    <>
      {pipe(
        plansByFamily,
        D.toPairs,
        A.map(([family, plans]) => (
          <SelectGroup key={family}>
            <SelectLabel className="SelectLabel">{family}</SelectLabel>
            {plans?.map(p => (
              <SelectItem key={p.externalId} value={p.externalId}>
                {p.name}
              </SelectItem>
            ))}
          </SelectGroup>
        ))
      )}
    </>
  )
}
