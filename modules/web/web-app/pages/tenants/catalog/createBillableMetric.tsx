import { FunctionComponent } from 'react'
import { useLocation } from 'react-router-dom'

import { ProductMetricsCreatePanel } from '@/features/productCatalog/metrics/ProductMetricsCreatePanel'

export const CreateBillableMetric: FunctionComponent = () => {
  const location = useLocation()
  const sourceMetricId = location.state?.sourceMetricId

  return (
    <>
      <ProductMetricsCreatePanel sourceMetricId={sourceMetricId} />
    </>
  )
}
