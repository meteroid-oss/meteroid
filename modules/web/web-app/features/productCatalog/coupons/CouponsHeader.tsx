import { EntityFilters, EntityHeader } from '@/features/TablePage'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

export const CouponsHeader = ({ children, count }: { children?: ReactNode; count?: number }) => {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Coupons"
        count={count}
        primaryAction={{ label: 'New coupon', onClick: () => navigate('add-coupon') }}
      />
      <EntityFilters>{children}</EntityFilters>
    </div>
  )
}
