import { colors, spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, Flex, Input2 } from '@ui/components'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/atoms/PageHeading/PageHeading'

interface SubscriptionsProps {
  count: number
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
}

export const SubscriptionsHeader: FunctionComponent<SubscriptionsProps> = ({
  count,
  isLoading,
  refetch,
  setEditPanelVisible,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading count={count}>Subscriptions</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="primary" onClick={() => setEditPanelVisible(true)}>
            <PlusIcon size={10} fill={colors.white1} /> New subscription
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <Input2
          placeholder="Search subscriptions"
          icon={<SearchIcon size={16} />}
          iconPosition="right"
          width="fit-content"
          disabled
        />
        <Button variant="tertiary" loading={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} />
        </Button>
      </Flex>
    </Flex>
  )
}
