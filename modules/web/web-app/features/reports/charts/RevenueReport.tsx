import {
  Button,
  Card,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { subYears } from 'date-fns'
import * as React from 'react'
import { useMemo } from 'react'
import { DateRange } from 'react-day-picker'

import { SimpleTable } from '@/components/table/SimpleTable'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { RevenueChart } from '@/features/dashboard/charts/RevenueChart'
import { useQuery } from '@/lib/connectrpc'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

const ALL_PLANS = '_all'

export const RevenueReport = () => {
  const defaultRange = useMemo(
    () => ({
      from: subYears(new Date(), 1),
      to: new Date(),
    }),
    []
  )
  const [range, setRange] = React.useState<DateRange | undefined>(defaultRange)
  const [plan, setPlan] = React.useState<string>(ALL_PLANS)

  const plans = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const selectedPlanIds = plan === ALL_PLANS ? [] : [plan]

  return (
    <>
      <div className=" pb-2">
        <div className="flex justify-between items-end flex-wrap pb-4">
          <h3 className="text-lg text-foreground font-medium">Revenue</h3>
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
          <div className="flex flex-row gap-1">
            <DatePickerWithRange range={range} setRange={setRange} />

            <Select onValueChange={setPlan} value={plan}>
              <SelectTrigger className="md:w-[180px]">
                <SelectValue placeholder="All plans" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL_PLANS}>All plans</SelectItem>
                {plans.data?.plans.map(p => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </div>
      <Card className="p-6">
        <RevenueChart
          from={range?.from ?? defaultRange.from}
          to={range?.to ?? defaultRange.to}
          plansId={selectedPlanIds}
        />
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
