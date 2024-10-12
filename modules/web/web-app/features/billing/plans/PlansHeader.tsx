import { spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

interface PlansHeaderProps {
  setEditPanelVisible: (visible: boolean) => void
  setSearch: (value: string | undefined) => void
}

export const PlansHeader: FunctionComponent<PlansHeaderProps> = ({
  setEditPanelVisible,
  setSearch,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading>Plans</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="primary" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> New plan
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder="Search plans"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          onChange={e => setSearch(e.target.value)}
        />
      </Flex>
    </Flex>
  )
}
