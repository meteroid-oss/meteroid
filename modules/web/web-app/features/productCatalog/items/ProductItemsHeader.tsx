import { spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

interface ProductItemsHeaderProps {
  heading: string
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
}

export const ProductItemsHeader: FunctionComponent<ProductItemsHeaderProps> = ({
  heading,
  isLoading,
  refetch,
  setEditPanelVisible,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading>{heading}</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button hasIcon variant="primary" onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> New product
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder={`Search ${heading.toLocaleLowerCase()}`}
          icon={<SearchIcon size={16} />}
          width="fit-content"
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </Flex>
    </Flex>
  )
}
