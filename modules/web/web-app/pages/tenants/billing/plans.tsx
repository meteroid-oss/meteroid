import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { FunctionComponent, useState } from 'react'

import { PlanCreateInitModal } from '@/features/billing/plans/PlanCreateInitModal'
import { PlansHeader } from '@/features/billing/plans/PlansHeader'
import { PlansTable } from '@/features/billing/plans/PlansTable'
import { useDebounce } from '@/hooks/useDebounce'

export const Plans: FunctionComponent = () => {
  const [createModalOpened, setCreateModalOpened] = useState(false)
  const [search, setSearch] = useDebounce<string | undefined>(undefined, 200)

  return (
    <Flex direction="column" gap={spaces.space9}>
      <PlansHeader setEditPanelVisible={() => setCreateModalOpened(true)} setSearch={setSearch} />
      <PlansTable search={search} />
      <PlanCreateInitModal
        modalVisible={createModalOpened}
        setModalVisible={setCreateModalOpened}
      />
    </Flex>
  )
}
