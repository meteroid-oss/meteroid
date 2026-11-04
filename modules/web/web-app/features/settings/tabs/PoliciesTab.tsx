import { Slot } from '@/edition'
import { MfaSettings } from '@/features/profile/MfaSettings'

export const PoliciesTab = () => (
  <div className="flex max-w-3xl flex-col gap-6">
    <MfaSettings />
    <Slot name="securitySettings.policies" />
  </div>
)
