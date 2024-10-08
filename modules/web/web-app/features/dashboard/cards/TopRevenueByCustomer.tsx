import { TopCard } from '@/features/dashboard/cards/TopCard'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { topRevenueByCustomer } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const TopRevenueByCustomers = () => {
  const q = useQuery(topRevenueByCustomer, { count: 6 })
  const { formatAmount } = useCurrency()

  return (
    <TopCard
      title="Top revenue by customers"
      loading={q.isLoading}
      values={q.data?.revenueByCustomer.map(customer => ({
        name: customer.customerName,
        value: formatAmount(customer.revenue),
        detailsPath: `customers/${customer.customerId}`,
      }))}
    />
  )
}
