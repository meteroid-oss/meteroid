import { TenantPageLayout } from '@/components/layouts'
import { DeadLetterDetail } from '@/features/admin/deadletter/DeadLetterDetail'

export const DeadLetterDetailPage = () => (
  <TenantPageLayout>
    <DeadLetterDetail />
  </TenantPageLayout>
)
