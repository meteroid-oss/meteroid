import { FunctionComponent } from 'react'

import { useNavigate } from 'react-router-dom'
import { ProductMetricsEditPanel } from '@/features/productCatalog/metrics/ProductMetricsEditPanel'

export const CreateBillableMetric: FunctionComponent = () => {
  const navigate = useNavigate()
  const closePanel = () => navigate('..')

  return (
    <>
      <ProductMetricsEditPanel visible closePanel={closePanel} />
    </>
  )
}
