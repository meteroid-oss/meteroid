import { TenantPageLayout } from '@/components/layouts'
import { DeadLetterList } from '@/features/admin/deadletter/DeadLetterList'

export const DeadLetterPage = () => (
  <TenantPageLayout>
    <DeadLetterList />
  </TenantPageLayout>
)
