import { disableQuery } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
} from '@md/ui'
import { subYears } from 'date-fns'
import * as React from 'react'
import { useMemo } from 'react'
import { DateRange } from 'react-day-picker'

import { SimpleTable } from '@/components/table/SimpleTable'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { MrrBreakdownCard } from '@/features/dashboard/cards/MrrBreakdownCard'
import { MrrLogsCard } from '@/features/dashboard/cards/MrrLogsCard'
import { MrrChart } from '@/features/dashboard/charts/MrrChart'
import { useQuery } from '@/lib/connectrpc'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

const ALL = '_all'

export const MrrReport = () => {
  const defaultRange = useMemo(
    () => ({
      from: subYears(new Date(), 1),
      to: new Date(),
    }),
    []
  )
  const [range, setRange] = React.useState<DateRange | undefined>(defaultRange)

  const [productFamily, setProductFamily] = React.useState<string>()

  // TODO multiselect https://github.com/mxkaske/mxkaske.dev/blob/main/components/craft/fancy-box.tsx
  const [plan, setPlan] = React.useState<string>()

  const productFamilies = useQuery(listProductFamilies)

  const plans = useQuery(
    listPlans,
    productFamily
      ? {
          sortBy: ListPlansRequest_SortBy.NAME_ASC,
          productFamilyLocalId: productFamily,
        }
      : disableQuery
  )

  React.useEffect(() => {
    if (productFamilies.data?.productFamilies[0]?.localId && productFamily !== ALL) {
      setProductFamily(productFamilies.data.productFamilies[0].localId)
    }
  }, [productFamilies.data?.productFamilies, productFamily])

  return (
    <>
      <div className=" pb-2">
        <div className="flex justify-between items-end flex-wrap pb-4">
          <h3 className="text-lg text-foreground font-medium">Monthly Recurring Revenue</h3>
          <div>
            <Button variant="primary">Save chart</Button>
          </div>
        </div>
        <div className="flex justify-between items-end flex-wrap">
          <div>
            <Button variant="link" className="p-0">
              Filter
            </Button>
          </div>
          <div className="flex flex-row   gap-1">
            <DatePickerWithRange range={range} setRange={setRange} />

            <Select onValueChange={setProductFamily} value={productFamily}>
              <SelectTrigger className="md:w-[180px]">
                <SelectValue placeholder="All product lines" />
              </SelectTrigger>
              <SelectContent>
                {productFamilies.data?.productFamilies.map(pf => (
                  <SelectItem key={pf.localId} value={pf.localId}>
                    {pf.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select onValueChange={setPlan} value={plan}>
              <SelectTrigger className="md:w-[180px]">
                <SelectValue placeholder="All plans" />
              </SelectTrigger>
              <SelectContent>
                {plans.data?.plans.map(p => (
                  <SelectItem key={p.localId} value={p.localId}>
                    {p.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </div>
      <Card className="p-6">
        <MrrChart
          from={range?.from ?? defaultRange.from}
          to={range?.to ?? defaultRange.to}
          plansId={plan ? [plan] : []}
        />
        <Separator className="m-2" />
        <div className="w-full flex flex-row h-[180px] relative">
          <MrrBreakdownCard />
          <Separator orientation="vertical" className="m-2" />
          <MrrLogsCard />
        </div>
      </Card>

      <div className="pt-4 pb-2 flex justify-between items-end flex-wrap">
        <h3 className="text-lg text-foreground font-medium">Table data</h3>
        <div>
          <Button variant="link">Export</Button>
        </div>
      </div>
      <Card className="p-4">
        <SimpleTable
          columns={[
            {
              header: ' ',
            },
            {
              header: ' ',
            },
          ]}
          data={[]}
          emptyMessage="Not implemented"
          containerClassName="max-h-xl"
        />
      </Card>
    </>
  )
}
