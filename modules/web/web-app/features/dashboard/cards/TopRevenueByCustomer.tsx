import { TopCard } from '@/features/dashboard/cards/TopCard'
import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { topRevenueByCustomer } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const TopRevenueByCustomers = () => {
  const q = useQuery(topRevenueByCustomer, { count: 6 })

  return (
    <TopCard
      title="Top revenue by customers"
      loading={q.isLoading}
      values={q.data?.revenueByCustomer.map(customer => ({
        name: customer.customerName,
        value: formatCurrency(customer.revenue),
        detailsPath: `customers/${customer.customerId}`,
      }))}
    />
  )
}
