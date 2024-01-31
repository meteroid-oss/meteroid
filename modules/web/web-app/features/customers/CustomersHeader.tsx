import { colors, spaces } from '@md/foundation'
import { ChevronDownIcon, PlusIcon, SearchIcon } from '@md/icons'
import { Button, Flex, Input2 } from '@ui/components'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/atoms/PageHeading/PageHeading'

interface CustomersProps {
  count: number
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (search: string) => void
  search: string
}

export const CustomersHeader: FunctionComponent<CustomersProps> = ({
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
        <PageHeading count={count}>Customers</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="tertiary">Import / Export</Button>
          <Button variant="primary" onClick={() => setEditPanelVisible(true)}>
            <PlusIcon size={10} fill={colors.white1} /> New customer
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <Input2
          placeholder="Search customers"
          icon={<SearchIcon size={16} />}
          iconPosition="right"
          width="fit-content"
          value={search}
          onChange={e => setSearch(e.target.value)}
        />
        <Button variant="tertiary">
          Show all <ChevronDownIcon size={14} />
        </Button>
        <Button variant="tertiary" loading={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} />
        </Button>
        <Button variant="tertiary" transparent>
          <PlusIcon size={12} />
          Filter
        </Button>
      </Flex>
    </Flex>
  )
}
