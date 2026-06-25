import { MfaSettings } from '@/features/profile/MfaSettings'

import type { FunctionComponent } from 'react'

export const UserSettings: FunctionComponent = () => {
  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <h1 className="text-2xl font-semibold">Account settings</h1>
      <MfaSettings />
    </div>
  )
}
