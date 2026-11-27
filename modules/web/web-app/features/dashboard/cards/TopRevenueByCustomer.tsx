import { Skeleton, cn } from '@md/ui'
import { Link } from 'react-router'

import { CustomerAvatar } from '@/features/customers/CustomerAvatar'
import { dashedLink } from '@/features/dashboard/cards/DashboardCard'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { topRevenueByCustomer } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

export const TopRevenueByCustomers = () => {
  const q = useQuery(topRevenueByCustomer, { count: 6 })
  const { formatAmount } = useCurrency()

  return (
    <div className="overflow-y-auto h-[180px] grow flex flex-col relative">
      <div className="text-[13px] leading-none tracking-[-0.13px] text-foreground/60 flex flex-row px-5 pt-5 pb-4 items-baseline w-full justify-between">
        Top revenue by customers
      </div>
      <div className="px-5 pb-5 space-y-2.5 relative">
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
            <div key={index} className="flex flex-row gap-3 items-center text-xs justify-between">
              <div className="flex flex-row items-center space-x-2.5 min-w-0 flex-1">
                {/* The customer's initials, as on their page (neutral, no per-customer colours). */}
                <CustomerAvatar name={customer.customerName} />
                <Link
                  to={`customers/${customer.customerId}`}
                  // 20px lines: `truncate` clips to the line box, the underline included.
                  className={cn('truncate leading-5', dashedLink)}
                >
                  {customer.customerName}
                </Link>
              </div>
              <div className="flex flex-col items-end shrink-0 text-right">
                <span className="text-sm font-medium">{formatAmount(customer.revenueAllTime)}</span>
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
