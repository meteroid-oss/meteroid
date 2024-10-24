import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { FunctionComponent } from 'react'

import { PlanCreateInitModal } from '@/features/billing/plans/PlanCreateInitModal'
import { PlansHeader } from '@/features/billing/plans/PlansHeader'
import { PlansTable } from '@/features/billing/plans/PlansTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'

export const Plans: FunctionComponent = () => {
  const [createModalOpened, setCreateModalOpened] = useQueryState('modal', false)

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)

  const debouncedSearch = useDebounceValue(search, 200)
  return (
    <Flex direction="column" gap={spaces.space9}>
      <PlansHeader
        setEditPanelVisible={() => setCreateModalOpened(true)}
        setSearch={setSearch}
        search={search}
      />
      <PlansTable search={debouncedSearch} />
      <PlanCreateInitModal
        modalVisible={createModalOpened}
        setModalVisible={setCreateModalOpened}
      />
    </Flex>
  )
}
