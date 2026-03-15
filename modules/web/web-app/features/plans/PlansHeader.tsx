import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'

export const PlansHeader = ({
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
        title="Plans"
        count={count}
        primaryAction={{ label: 'New plan', onClick: () => navigate('add-plan') }}
      />
      <EntityFilters isLoading={isLoading} refetch={refetch}>
        {children}
      </EntityFilters>
    </div>
  )
}
