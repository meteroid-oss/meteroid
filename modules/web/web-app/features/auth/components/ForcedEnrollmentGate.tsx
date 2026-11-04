import { Button } from '@md/ui'
import { Link } from 'react-router-dom'

import { MfaSettings } from '@/features/profile/MfaSettings'

/// Full-screen blocker shown when the user's org requires MFA but the user
/// has not yet enrolled. No navigation is possible — the only escape is to
/// finish enrolling (which clears `mfa_enrollment_required` in the next `me`
/// response) or to sign out.
export const ForcedEnrollmentGate = () => {
  return (
    <div className="fixed inset-0 z-50 overflow-y-auto bg-background">
      <div className="mx-auto flex min-h-full max-w-2xl flex-col gap-6 px-6 py-12">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold">Two-factor authentication required</h1>
            <p className="mt-2 text-sm text-muted-foreground">
              Your organization requires every member to enable two-factor authentication. Finish
              setup to continue using Meteroid.
            </p>
          </div>
          <Button variant="secondary" asChild>
            <Link to="/logout">Sign out</Link>
          </Button>
        </div>
        <MfaSettings />
      </div>
    </div>
  )
}
