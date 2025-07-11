import { useQuery } from '@connectrpc/connect-query'

import { StatCard } from '@/features/dashboard/cards/StatCard'
import { generalStats } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const TopSection = () => {
  const stats = useQuery(generalStats)
  // const { formatAmount } = useCurrency()

  return (
    <div className="flex flex-row  flex-wrap md:flex-nowrap items-center gap-4 ml-auto ">
      <StatCard
        title="Signups"
        loading={!stats.isFetched}
        value={stats.data?.signups?.count?.toString() ?? 'No data'}
        // trend={formattedTrend(stats.data?.totalNetRevenue?.trend)}
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
        // secondaryValue={formatAmount(stats.data?.pendingInvoices?.valueCents)}
      />
    </div>
  )
}
