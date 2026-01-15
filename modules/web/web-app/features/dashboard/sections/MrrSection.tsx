import {
  Card,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
} from '@md/ui'
import * as React from 'react'
import { useMemo, useState } from 'react'

import {
  DatePresetSelect,
  DateRangePreset,
  getDateRangeFromPreset,
} from '@/features/dashboard/DatePresetSelect'
import { MrrBreakdownCard } from '@/features/dashboard/cards/MrrBreakdownCard'
import { MrrLogsCard } from '@/features/dashboard/cards/MrrLogsCard'
import { MrrChart } from '@/features/dashboard/charts/MrrChart'
import { RevenueChart } from '@/features/dashboard/charts/RevenueChart'
import { ChartType } from '@/features/dashboard/charts/types'
import { useQuery } from '@/lib/connectrpc'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

const ALL_PLANS = '_all'

export const MrrSection = () => {
  const [datePreset, setDatePreset] = useState<DateRangePreset>('last30days')
  const range = useMemo(() => getDateRangeFromPreset(datePreset), [datePreset])
  const [chartType, setChartType] = useState<ChartType>('revenue')

  const [plan, setPlan] = React.useState<string>(ALL_PLANS)

  const plans = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const selectedPlanIds = plan === ALL_PLANS ? [] : [plan]

  return (
    <>
      <div className="pt-2 pb-2">
        <div className="flex justify-between items-center flex-wrap gap-2">
          <h3 className="text-lg text-muted-foreground font-medium">Overview</h3>
          <div className="flex flex-row gap-1">
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

            <DatePresetSelect value={datePreset} onChange={setDatePreset} />
          </div>
        </div>
      </div>
      <Card className="p-6">
        {chartType === 'revenue' ? (
          <RevenueChart
            from={range.from}
            to={range.to}
            plansId={selectedPlanIds}
            chartType={chartType}
            onChartTypeChange={setChartType}
          />
        ) : (
          <MrrChart
            from={range.from}
            to={range.to}
            plansId={selectedPlanIds}
            chartType={chartType}
            onChartTypeChange={setChartType}
          />
        )}
        <Separator className="m-2" />
        <div className="w-full flex flex-row h-[180px] relative">
          <MrrBreakdownCard from={range.from} to={range.to} />
          <Separator orientation="vertical" className="m-2" />
          <MrrLogsCard />
        </div>
      </Card>
    </>
  )
}
