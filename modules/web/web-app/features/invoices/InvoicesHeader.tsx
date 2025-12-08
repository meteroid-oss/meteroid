import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'
import { Link } from 'react-router-dom'

import PageHeading from '@/components/PageHeading/PageHeading'
import { FilterDropdown } from '@/features/invoices/FilterDropdown'
import { InvoicesSearch } from '@/features/invoices/types'

type InvoicesProps = {
  count: number
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (search: InvoicesSearch) => void
  search: InvoicesSearch
}

export const InvoicesHeader: FunctionComponent<InvoicesProps> = ({
  count,
  isLoading,
  refetch,
  setSearch,
  search,
}) => {
  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading count={count}>Invoices</PageHeading>
        <div className="flex flex-row gap-2">
          <Button variant="secondary" disabled size="sm">
            Import / Export
          </Button>
          <Link to="create">
            <Button variant="primary" size="sm">
              <PlusIcon size={10} fill="white"/> New invoice
            </Button>
          </Link>
        </div>
      </div>
      <div className="flex flex-row items-center gap-2">
        <InputWithIcon
          placeholder="Search by customer"
          icon={<SearchIcon size={16}/>}
          width="fit-content"
          value={search.text}
          onChange={e => setSearch({ ...search, text: e.target.value })}
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''}/>
        </Button>
        <FilterDropdown
          status={search.status}
          setStatus={value => setSearch({ ...search, status: value })}
        />
      </div>
    </div>
  )
}
