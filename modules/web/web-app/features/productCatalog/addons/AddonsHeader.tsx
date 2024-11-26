import { EntityFilters, EntityHeader } from '@/features/TablePage'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

export const AddonsHeader = ({ children, count }: { children?: ReactNode; count?: number }) => {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Addons"
        count={count}
        primaryAction={{ label: 'New addon', onClick: () => navigate('add-addon') }}
      />
      <EntityFilters>{children}</EntityFilters>
    </div>
  )
}
