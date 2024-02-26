import { SparklineCard } from '@/features/dashboard/cards/SparklineCard'
import { TopRevenueByCustomers } from '@/features/dashboard/cards/TopRevenueByCustomer'
import { SignupsSparkline } from '@/features/dashboard/charts/SignupsSparkline'
import { TrialConversionSparkline } from '@/features/dashboard/charts/TrialConversionSparkline'
import { formatRate } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { generalStats } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const DetailsSection = () => {
  const stats = useQuery(generalStats)

  return (
    <div className="container py-4">
      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-x-0 gap-y-4">
        <SparklineCard
          className="md:!pl-0 md:!border-l-0 "
          title="New customers"
          value={stats.data?.signups?.count?.toString() ?? 'No data'}
          chart={<SignupsSparkline />}
          // secondaryValue="11%"
        />
        <SparklineCard
          title="Trial conversion rate"
          value={formatRate(stats.data?.trialConversion?.ratePercent) ?? 'No data'}
          //   secondaryValue=""
          chart={<TrialConversionSparkline />}
        />
        <TopRevenueByCustomers />
      </div>
    </div>
  )
}
