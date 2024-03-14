import { spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

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
  setEditPanelVisible,
  setSearch,
  search,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading count={count}>Invoices</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="secondary" disabled size="sm">
            Import / Export
          </Button>
          <Button variant="primary" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> New invoice
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder="Search by customer"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          value={search.text}
          onChange={e => setSearch({ ...search, text: e.target.value })}
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
        <FilterDropdown
          status={search.status}
          setStatus={value => setSearch({ ...search, status: value })}
        />
      </Flex>
    </Flex>
  )
}
