import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'

export const AddonsHeader = ({
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
        title="Addons"
        count={count}
        primaryAction={{ label: 'New addon', onClick: () => navigate('add-addon') }}
      />
      <EntityFilters isLoading={isLoading} refetch={refetch}>
        {children}
      </EntityFilters>
    </div>
  )
}
