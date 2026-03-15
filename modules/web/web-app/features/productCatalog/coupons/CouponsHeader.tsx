import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'

export const CouponsHeader = ({
  children,
  count,
  isLoading,
  refetch,
}: {
  children?: ReactNode
  count?: number
  isLoading?: boolean
  refetch?: () => void
}) => {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Coupons"
        count={count}
        primaryAction={{ label: 'New coupon', onClick: () => navigate('add-coupon') }}
      />
      <EntityFilters isLoading={isLoading} refetch={refetch}>
        {children}
      </EntityFilters>
    </div>
  )
}
