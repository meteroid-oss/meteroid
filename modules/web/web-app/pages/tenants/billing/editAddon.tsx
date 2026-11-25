import { FunctionComponent } from 'react'
import { Navigate, useParams } from 'react-router'

/** An add-on is edited in its panel beside the catalog: older links open it there. */
export const EditAddon: FunctionComponent = () => {
  const { addonId } = useParams<{ addonId: string }>()
  return <Navigate to={`../${addonId ?? ''}`} replace />
}
