import { FunctionComponent } from 'react'
import { Navigate } from 'react-router'

/** Add-ons are offered from the plan's "Add-ons" section: older links open it. */
export const CreateAddOn: FunctionComponent = () => (
  <Navigate to={{ pathname: '..', search: '?more=addons' }} replace />
)
