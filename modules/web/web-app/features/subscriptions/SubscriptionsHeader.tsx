import { colors, spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@ui/components'
import { Flex } from '@ui/components/legacy'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'
import { Link } from 'react-router-dom'

import PageHeading from '@/components/PageHeading/PageHeading'

interface SubscriptionsProps {
  count: number
  isLoading: boolean
  refetch: () => void
}

export const SubscriptionsHeader: FunctionComponent<SubscriptionsProps> = ({
  count,
  isLoading,
  refetch,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading count={count}>Subscriptions</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Link to="create">
            <Button variant="primary">
              <PlusIcon size={10} fill={colors.white1} /> New subscription
            </Button>
          </Link>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder="Search subscriptions"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          disabled
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </Flex>
    </Flex>
  )
}
