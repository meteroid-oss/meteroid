import { SearchIcon } from '@md/icons'
import { Button, InputWithIcon, Tabs, TabsList, TabsTrigger, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { FileUpIcon, PlusIcon, RefreshCwIcon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { CustomersExportModal } from '@/features/customers/modals/CustomersExportModal'
import { CustomersImportModal } from '@/features/customers/modals/CustomersImportModal'

interface CustomersHeaderProps {
  count?: number
  isLoading?: boolean
  refetch?: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (search: string) => void
  search: string
  onImportSuccess?: () => void
}

export const CustomersHeader: FunctionComponent<CustomersHeaderProps> = ({
  count,
  isLoading,
  refetch,
  setEditPanelVisible,
  setSearch,
  search,
  onImportSuccess,
}) => {
  const [searchParams, setSearchParams] = useSearchParams()
  const currentTab = searchParams.get('tab') || 'active'

  const [visible, setVisible] = useState(false)
  const [importVisible, setImportVisible] = useState(false)

  const updateTab = (tab: string) => {
    const newSearchParams = new URLSearchParams(searchParams)
    if (tab === 'active') newSearchParams.delete('tab')
    else newSearchParams.set('tab', tab.toLowerCase())
    setSearchParams(newSearchParams)
  }

  return (
    <>
      <div className="flex flex-col gap-8">
        <div className="flex flex-row items-center justify-between">
          <h1 className="text-2xl font-bold">
            Customers{' '}
            {count !== undefined && (
              <span className="text-xs font-medium text-muted-foreground">({count})</span>
            )}
          </h1>
          <div className="flex flex-row gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <span>
                  <Button size="sm" variant="secondary" disabled>
                    Export
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent>Coming soon</TooltipContent>
            </Tooltip>
            <Button variant="secondary" size="sm" onClick={() => setImportVisible(true)}>
              <FileUpIcon className="h-4 w-4 mr-2" />
              Import CSV
            </Button>
            <Button size="sm" variant="default" hasIcon onClick={() => setEditPanelVisible(true)}>
              <PlusIcon className="w-4 h-4" /> New customer
            </Button>
          </div>
        </div>
        <div className="flex flex-row items-center justify-between">
          <div className="flex flex-row items-center gap-2">
            <InputWithIcon
              placeholder="Search..."
              icon={<SearchIcon size={16} />}
              width="fit-content"
              value={search}
              onChange={e => setSearch(e.target.value)}
            />
            <Tabs value={currentTab} onValueChange={updateTab}>
              <TabsList>
                <TabsTrigger value="active">Active</TabsTrigger>
                <TabsTrigger value="archived">Archived</TabsTrigger>
              </TabsList>
            </Tabs>
          </div>
          {refetch && (
            <Button variant="outline" size="sm" disabled={isLoading} onClick={refetch}>
              <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
            </Button>
          )}
        </div>
      </div>
      <CustomersExportModal openState={[visible, setVisible]} />
      <CustomersImportModal
        openState={[importVisible, setImportVisible]}
        onSuccess={onImportSuccess}
      />
    </>
  )
}

