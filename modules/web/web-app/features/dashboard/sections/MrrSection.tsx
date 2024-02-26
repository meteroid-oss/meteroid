import { disableQuery } from '@connectrpc/connect-query'
import { Select, SelectItem } from '@ui/components'
import { subYears } from 'date-fns'
import { useMemo } from 'react'
import * as React from 'react'
import { DateRange } from 'react-day-picker'

import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { MrrBreakdownCard } from '@/features/dashboard/cards/MrrBreakdownCard'
import { MrrLogsCard } from '@/features/dashboard/cards/MrrLogsCard'
import { MrrChart } from '@/features/dashboard/charts/MrrChart'
import { useQuery } from '@/lib/connectrpc'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

const ALL = '_all'

export const MrrSection = () => {
  const defaultRange = useMemo(
    () => ({
      from: subYears(new Date(), 1),
      to: new Date(),
    }),
    []
  )
  const [range, setRange] = React.useState<DateRange | undefined>(defaultRange)

  const [productFamily, setProductFamily] = React.useState<string>(ALL)

  // TODO multiselect https://github.com/mxkaske/mxkaske.dev/blob/main/components/craft/fancy-box.tsx
  const [plan, setPlan] = React.useState<string>(ALL)

  const productFamilies = useQuery(listProductFamilies)

  const plans = useQuery(
    listPlans,
    productFamily !== ALL
      ? {
          orderBy: ListPlansRequest_SortBy.NAME_ASC,
          productFamilyExternalId: productFamily,
        }
      : disableQuery
  )

  React.useEffect(() => {
    if (productFamilies.data?.productFamilies[0]?.externalId && productFamily !== ALL) {
      setProductFamily(productFamilies.data.productFamilies[0].externalId)
    }
  }, [productFamilies.data?.productFamilies, productFamily])

  return (
    <>
      <div className="pt-2 pb-2 border-b border-slate-400">
        <div className="flex justify-between">
          <h3 className=" text-lg">Your overview</h3>
          <div className="flex gap-1">
            <DatePickerWithRange range={range} setRange={setRange} />
            <Select onValueChange={setProductFamily} value={productFamily}>
              <SelectItem value="_all">All product lines</SelectItem>
              {productFamilies.data?.productFamilies.map(pf => (
                <SelectItem key={pf.externalId} value={pf.externalId}>
                  {pf.name}
                </SelectItem>
              ))}
            </Select>
            <Select onValueChange={setPlan} value={plan}>
              <SelectItem value="_all">All plans</SelectItem>
              {plans.data?.plans.map(p => (
                <SelectItem key={p.externalId} value={p.externalId}>
                  {p.name}
                </SelectItem>
              ))}
            </Select>
          </div>
        </div>
      </div>
      <div>
        <MrrChart
          from={range?.from ?? defaultRange.from}
          to={range?.to ?? defaultRange.to}
          plansId={plan !== ALL ? [plan] : []}
        />
      </div>
      <div className="w-full flex flex-row">
        <MrrBreakdownCard />
        <MrrLogsCard />
      </div>
    </>
  )
}
