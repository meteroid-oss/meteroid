import { Skeleton, cn } from '@md/ui'
import { UserRoundIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { topRevenueByCustomer } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

const colors = [
  'bg-red-700',
  'bg-purple-700',
  'bg-slate-500',
  'bg-indigo-700',
  'bg-blue-700',
  'bg-green-700',
  'bg-yellow-700',
]

const getColor = (key: string) => {
  const hash = key.split('').reduce((acc, char) => acc + char.charCodeAt(0), 0)
  return colors[hash % colors.length]
}

export const TopRevenueByCustomers = () => {
  const q = useQuery(topRevenueByCustomer, { count: 6 })
  const { formatAmount } = useCurrency()

  return (
    <div className="overflow-y-auto h-[180px] grow flex flex-col relative">
      <div className="text-sm font-semibold flex flex-row px-6 py-4 items-baseline w-full justify-between">
        Top revenue by customers
      </div>
      <div className="px-6 pb-4 space-y-2.5 relative">
        {q.isLoading ? (
          [1, 2, 3].map((_, index) => (
            <div key={index} className="flex flex-row gap-4 items-center text-xs justify-between">
              <Skeleton className="w-[40%]" height="1.2rem" />
              <Skeleton className="grow" height="1.2rem" />
            </div>
          ))
        ) : !q.data?.revenueByCustomer?.length ? (
          <div className="h-[74px] text-center font-semibold text-sm">no data</div>
        ) : (
          q.data.revenueByCustomer.map((customer, index) => (
            <div
              key={index}
              className="flex flex-row gap-3 items-center text-xs justify-between"
            >
              <div className="flex flex-row items-center space-x-2.5 min-w-0 flex-1">
                <div
                  className={cn(
                    'p-1.5 flex items-center justify-center rounded-sm text-primary-foreground shrink-0',
                    getColor(customer.customerName)
                  )}
                >
                  <UserRoundIcon size={12} />
                </div>
                <Link
                  to={`customers/${customer.customerId}`}
                  className="truncate underline decoration-foreground decoration-dashed underline-offset-4"
                >
                  {customer.customerName}
                </Link>
              </div>
              <div className="flex flex-col items-end shrink-0 text-right">
                <span className="text-sm font-medium">
                  {formatAmount(customer.revenueAllTime)}
                </span>
                <span className="text-[10px] text-muted-foreground">
                  {formatAmount(customer.revenueYtd)} YTD
                </span>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}
