import { colors, spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@ui2/components'
import { Flex } from '@ui2/components/legacy'
import { LoaderIcon, RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

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
          <Button disabled variant="secondary">
            Import / Export
          </Button>
          <Button hasIcon variant="default" onClick={() => setEditPanelVisible(true)}>
            <PlusIcon size={10} fill={colors.white1} /> New customer
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder="Search customers"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          value={search}
          onChange={e => setSearch(e.target.value)}
        />
        <Button variant="alternative" onClick={refetch}>
          {isLoading ? <LoaderIcon size={14} /> : <RefreshCwIcon size={14} />}
        </Button>
      </Flex>
    </Flex>
  )
}
