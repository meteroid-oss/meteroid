import { useQuery } from '@connectrpc/connect-query'

import { StatCard } from '@/features/dashboard/cards/StatCard'
import { formatCurrency, formattedTrend } from '@/features/dashboard/utils'
import { generalStats } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const TopSection = () => {
  const stats = useQuery(generalStats)

  return (
    <div className="flex flex-row  flex-wrap md:flex-nowrap items-center gap-4 ml-auto ">
      <StatCard
        title="Total net revenue"
        loading={!stats.isFetched}
        value={formatCurrency(stats.data?.totalNetRevenue?.trend?.current)}
        secondaryValue="YTD"
        trend={formattedTrend(stats.data?.totalNetRevenue?.trend)}
      />
      <StatCard
        title="Active subscriptions"
        detailPath="subscriptions"
        value={stats.data?.totalActiveSubscriptions?.count?.toString() ?? 'No data'}
        loading={!stats}
      />
      <StatCard
        title="Pending invoices"
        detailPath="invoices"
        value={stats.data?.pendingInvoices?.count?.toString() ?? 'No data'}
        loading={!stats}
        secondaryValue={formatCurrency(stats.data?.pendingInvoices?.valueCents)}
      />
    </div>
  )
}
