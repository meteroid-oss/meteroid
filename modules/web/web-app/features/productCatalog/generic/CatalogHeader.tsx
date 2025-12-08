import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

interface CatalogHeaderProps {
  heading: string
  newButtonText: string
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (value: string) => void
}

export const CatalogHeader: FunctionComponent<CatalogHeaderProps> = ({
  heading,
  newButtonText,
  isLoading,
  refetch,
  setEditPanelVisible,
  setSearch,
}) => {
  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading>{heading}</PageHeading>
        <div className="flex flex-row gap-2">
          <Button hasIcon variant="primary" onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> {newButtonText}
          </Button>
        </div>
      </div>
      <div className="flex flex-row items-center gap-2">
        <InputWithIcon
          placeholder={`Search ${heading.toLocaleLowerCase()}`}
          icon={<SearchIcon size={16} />}
          width="fit-content"
          onChange={e => setSearch(e.target.value)}
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </div>
    </div>
  )
}
