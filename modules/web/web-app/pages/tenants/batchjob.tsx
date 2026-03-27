import { TenantPageLayout } from '@/components/layouts'
import { BatchJobDetail } from '@/features/batch-jobs/BatchJobDetail'
import { useTypedParams } from '@/utils/params'

import type { FunctionComponent } from 'react'

export const BatchJobPage: FunctionComponent = () => {
  const { batchJobId } = useTypedParams<{ batchJobId: string }>()

  return (
    <TenantPageLayout>
      <BatchJobDetail jobId={batchJobId!} />
    </TenantPageLayout>
  )
}
