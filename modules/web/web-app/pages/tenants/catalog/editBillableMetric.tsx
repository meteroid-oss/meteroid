import { FunctionComponent } from 'react'
import { useParams } from 'react-router-dom'

import { ProductMetricsEditView } from '@/features/productCatalog/metrics/ProductMetricsEditView'

export const EditBillableMetric: FunctionComponent = () => {
  const { metricId } = useParams<{ metricId: string }>()

  if (!metricId) {
    return null
  }

  return <ProductMetricsEditView metricId={metricId} />
}
