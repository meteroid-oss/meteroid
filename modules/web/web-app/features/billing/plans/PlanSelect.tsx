import { Input, SelectContent, SelectItem, SelectRoot, SelectTrigger } from '@ui/components'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { Plan } from '@/rpc/api/plans/v1/models_pb'
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
    <SelectRoot value={value} onValueChange={onValueChange}>
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
    </SelectRoot>
  )
}

const PlanItems = ({ search }: { search: string }) => {
  const query = useQuery(listPlans, {
    pagination: {
      limit: 20,
      offset: 0,
    },
    productFamilyExternalId: 'default',
    search: search.length > 0 ? search : undefined,
    orderBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const plans = query.data?.plans || []

  return (
    <>
      {plans.map(p => (
        <SelectItem key={p.externalId} value={p.externalId}>
          {p.name}
        </SelectItem>
      ))}
    </>
  )
}
