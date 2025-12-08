import { PlusIcon, SearchIcon } from '@md/icons'
import {
  Button,
  InputWithIcon,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

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
  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading count={totalCount}>Metrics</PageHeading>
        <div className="flex flex-row gap-2">
          <Button variant="primary" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> New metric
          </Button>
        </div>
      </div>
      <div className="flex flex-row items-center gap-2">
        <InputWithIcon
          placeholder="Search metrics"
          icon={<SearchIcon size={16} />}
          width="fit-content"
        />
        <Select value={statusFilter} onValueChange={onStatusFilterChange}>
          <SelectTrigger className="w-[140px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="active">Active</SelectItem>
            <SelectItem value="archived">Archived</SelectItem>
          </SelectContent>
        </Select>
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </div>
    </div>
  )
}
