import { colors, spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, Flex, Input2 } from '@ui/components'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/atoms/PageHeading/PageHeading'

interface PlansHeaderProps {
  setEditPanelVisible: (visible: boolean) => void
}

export const PlansHeader: FunctionComponent<PlansHeaderProps> = ({ setEditPanelVisible }) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading>Plans</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="primary" onClick={() => setEditPanelVisible(true)}>
            <PlusIcon size={10} fill={colors.white1} /> New plan
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <Input2
          placeholder="Search plans"
          icon={<SearchIcon size={16} />}
          iconPosition="right"
          width="fit-content"
        />
      </Flex>
    </Flex>
  )
}
