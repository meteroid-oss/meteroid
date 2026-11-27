import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import * as React from 'react'
import { useMemo, useState } from 'react'

import {
  DASHBOARD_FILTER_TRIGGER,
  DatePresetSelect,
  DateRangePreset,
  getDateRangeFromPreset,
} from '@/features/dashboard/DatePresetSelect'
import { DashboardCard } from '@/features/dashboard/cards/DashboardCard'
import { MrrBreakdownCard } from '@/features/dashboard/cards/MrrBreakdownCard'
import { MrrLogsCard } from '@/features/dashboard/cards/MrrLogsCard'
import { MrrChart } from '@/features/dashboard/charts/MrrChart'
import { RevenueChart } from '@/features/dashboard/charts/RevenueChart'
import { ChartType } from '@/features/dashboard/charts/types'
import { useQuery } from '@/lib/connectrpc'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

const ALL_PLANS = '_all'

export const MrrSection = () => {
  const [datePreset, setDatePreset] = useState<DateRangePreset>('last90days')
  const range = useMemo(() => getDateRangeFromPreset(datePreset), [datePreset])
  const [chartType, setChartType] = useState<ChartType>('revenue')

  const [plan, setPlan] = React.useState<string>(ALL_PLANS)

  const plans = useQuery(listPlans, {})

  const selectedPlanIds = plan === ALL_PLANS ? [] : [plan]

  return (
    <div className="flex flex-col gap-4">
      <DashboardCard>
        {/* The chart draws its own title row; the filters sit on that row's right end. */}
        <div className="absolute top-[14px] right-5 z-10 flex items-center gap-2">
          <Select onValueChange={setPlan} value={plan}>
            <SelectTrigger className={DASHBOARD_FILTER_TRIGGER}>
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
      </DashboardCard>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <MrrBreakdownCard from={range.from} to={range.to} />
        <MrrLogsCard />
      </div>
    </div>
  )
}
