import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'

export const FeaturesHeader = ({
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
        title="Features"
        count={count}
        beta
        primaryAction={{ label: 'New feature', onClick: () => navigate('create') }}
      />
      <EntityFilters isLoading={isLoading} refetch={refetch}>
        {children}
      </EntityFilters>
    </div>
  )
}
