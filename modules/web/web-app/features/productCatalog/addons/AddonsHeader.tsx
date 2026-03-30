import { SearchIcon } from '@md/icons'
import { InputWithIcon } from '@md/ui'
import { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'

import { EntityFilters, EntityHeader } from '@/features/TablePage'

export const AddonsHeader = ({
  children,
  count,
  search,
  setSearch,
}: {
  children?: ReactNode
  count?: number
  search?: string
  setSearch?: (value: string) => void
}) => {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col gap-6">
      <EntityHeader
        title="Addons"
        count={count}
        primaryAction={{ label: 'New addon', onClick: () => navigate('add-addon') }}
      />
      <EntityFilters>
        {setSearch && (
          <InputWithIcon
            placeholder="Search addons"
            icon={<SearchIcon size={16} />}
            value={search}
            onChange={e => setSearch(e.target.value)}
            width="fit-content"
          />
        )}
        {children}
      </EntityFilters>
    </div>
  )
}
