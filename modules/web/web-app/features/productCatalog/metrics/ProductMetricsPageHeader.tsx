import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'
import { BaseFilter } from '@/features/TablePage'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'

interface MetricsHeaderProps {
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  statusFilter: 'all' | 'active' | 'archived'
  onStatusFilterChange: (status: 'all' | 'active' | 'archived') => void
  totalCount: number
}

export const ProductMetricsPageHeader: FunctionComponent<MetricsHeaderProps> = ({
  isLoading,
  refetch,
  setEditPanelVisible,
  statusFilter,
  onStatusFilterChange,
  totalCount,
}) => {
  const isExpress = useIsExpressOrganization()

  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading count={totalCount}>Metrics</PageHeading>
        {!isExpress && (
          <div className="flex flex-row gap-2">
            <Button variant="primary" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
              <PlusIcon size={10} /> New metric
            </Button>
          </div>
        )}
      </div>
      <div className="flex flex-row items-center justify-between">
        <div className="flex flex-row items-center gap-2">
          <InputWithIcon
            placeholder="Search metrics"
            icon={<SearchIcon size={16} />}
            width="fit-content"
          />
          <BaseFilter
            entries={[
              { label: 'Active', value: 'active' },
              { label: 'Archived', value: 'archived' },
            ]}
            emptyLabel="All"
            selected={statusFilter !== 'all' ? [statusFilter] : []}
            onSelectionChange={(value, checked) =>
              onStatusFilterChange(checked ? (value as 'active' | 'archived') : 'all')
            }
          />
        </div>
        <Button variant="outline" size="sm" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </div>
    </div>
  )
}
