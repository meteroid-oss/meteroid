import { spaces } from '@md/foundation'
import { Flex } from '@ui/components'
import { FunctionComponent, useState } from 'react'

import { PlanCreateInitModal } from '@/features/billing/plans/PlanCreateInitModal'
import { PlansHeader } from '@/features/billing/plans/PlansHeader'
import { PlansTable } from '@/features/billing/plans/PlansTable'

export const Plans: FunctionComponent = () => {
  const [createModalOpened, setCreateModalOpened] = useState(false)

  return (
    <Flex direction="column" gap={spaces.space9}>
      <PlansHeader setEditPanelVisible={() => setCreateModalOpened(true)} />
      <PlansTable />
      <PlanCreateInitModal
        modalVisible={createModalOpened}
        setModalVisible={setCreateModalOpened}
      />
    </Flex>
  )
}
