import { EntityFilters, EntityHeader } from '@/features/TablePage'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

export const PlansHeader = ({ children, count }: { children?: ReactNode; count?: number }) => {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Plans"
        count={count}
        primaryAction={{ label: 'New plan', onClick: () => navigate('add-plan') }}
        secondaryActions={[{ label: 'Placeholder', onClick: () => 1 }]}
      />
      <EntityFilters>{children}</EntityFilters>
    </div>
  )
}
