import { colors, spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, Flex, Input2 } from '@ui/components'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/atoms/PageHeading/PageHeading'
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
          <Button variant="tertiary" disabled>
            Import / Export
          </Button>
          <Button variant="primary" onClick={() => setEditPanelVisible(true)}>
            <PlusIcon size={10} fill={colors.white1} /> New invoice
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <Input2
          placeholder="Search by customer"
          icon={<SearchIcon size={16} />}
          iconPosition="right"
          width="fit-content"
          value={search.text}
          onChange={e => setSearch({ ...search, text: e.target.value })}
        />
        <Button variant="tertiary" loading={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} />
        </Button>
        <FilterDropdown
          status={search.status}
          setStatus={value => setSearch({ ...search, status: value })}
        />
      </Flex>
    </Flex>
  )
}
