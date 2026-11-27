import { TenantPageLayout } from '@/components/layouts'
import { DetailPageLayout } from '@/components/layouts/ListPageLayout'
import { BatchJobDetail } from '@/features/batch-jobs/BatchJobDetail'
import { useBasePath } from '@/hooks/useBasePath'
import { useTypedParams } from '@/utils/params'

import type { FunctionComponent } from 'react'

export const BatchJobPage: FunctionComponent = () => {
  const { batchJobId } = useTypedParams<{ batchJobId: string }>()
  const basePath = useBasePath()

  return (
    <TenantPageLayout>
      <DetailPageLayout
        section={{ label: 'Batch jobs', to: `${basePath}/settings/batch-jobs` }}
        title="Batch job"
      >
        <BatchJobDetail jobId={batchJobId!} hideBack />
      </DetailPageLayout>
    </TenantPageLayout>
  )
}
