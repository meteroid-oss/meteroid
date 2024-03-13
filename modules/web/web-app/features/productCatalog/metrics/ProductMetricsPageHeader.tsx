import { spaces } from '@md/foundation'
import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'

interface MetricsHeaderProps {
  isLoading: boolean
  refetch: () => void
  setEditPanelVisible: (visible: boolean) => void
}

export const ProductMetricsPageHeader: FunctionComponent<MetricsHeaderProps> = ({
  isLoading,
  refetch,
  setEditPanelVisible,
}) => {
  return (
    <Flex direction="column" gap={spaces.space9}>
      <Flex direction="row" align="center" justify="space-between">
        <PageHeading>Metrics</PageHeading>
        <Flex direction="row" gap={spaces.space4}>
          <Button variant="alternative" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
            <PlusIcon size={10} /> New metric
          </Button>
        </Flex>
      </Flex>
      <Flex direction="row" align="center" gap={spaces.space4}>
        <InputWithIcon
          placeholder="Search metrics"
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
