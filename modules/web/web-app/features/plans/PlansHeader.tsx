import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'

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
  const isExpress = useIsExpressOrganization()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Plans"
        count={count}
        primaryAction={isExpress ? undefined : { label: 'New plan', onClick: () => navigate('add-plan') }}
      />
      <EntityFilters isLoading={isLoading} refetch={refetch}>
        {children}
      </EntityFilters>
    </div>
  )
}
