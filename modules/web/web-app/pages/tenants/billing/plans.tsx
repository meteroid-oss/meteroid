import { spaces } from '@md/foundation'
import { Flex } from '@ui/components'
import { FunctionComponent, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { PlanCreateInitModal } from '@/features/billing/plans/PlanCreateInitModal'
import { PlansHeader } from '@/features/billing/plans/PlansHeader'
import { PlansTable } from '@/features/billing/plans/PlansTable'

export const Plans: FunctionComponent = () => {
  const navigate = useNavigate()

  // const setEditPanelVisible = () => navigate('new')

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
