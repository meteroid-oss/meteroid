import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listCustomersByIds } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import { audienceLeaves, type AudienceLabelContext } from './model'

import type { AudiencePredicate } from './types'

/** Plans, customer property definitions (archived included) and the names labels need. */
export const useAudienceNames = (predicates: AudiencePredicate[]) => {
  const plansData = useQuery(listPlans, {}).data?.plans
  const plans = useMemo(() => plansData ?? [], [plansData])

  const definitionsData = useQuery(listCustomPropertyDefinitions, {
    entityType: CustomPropertyEntityType.CUSTOMER,
    includeArchived: true,
  }).data?.definitions
  const definitions = useMemo(() => definitionsData ?? [], [definitionsData])

  const customerIds = useMemo(
    () =>
      [
        ...new Set(
          audienceLeaves(predicates).flatMap(l => ('customer' in l ? l.customer.customer_ids : []))
        ),
      ].sort(),
    [predicates]
  )
  const customersData = useQuery(
    listCustomersByIds,
    { ids: customerIds },
    { enabled: customerIds.length > 0 }
  ).data?.customers

  const ctx: AudienceLabelContext = useMemo(
    () => ({
      planNames: new Map(plans.map(p => [p.id, p.name])),
      customerNames: new Map((customersData ?? []).map(c => [c.id, c.name])),
      propertyNames: new Map(definitions.map(d => [d.key, d.name])),
    }),
    [plans, customersData, definitions]
  )

  return { plans, definitions, ctx }
}
