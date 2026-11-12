import { Spinner } from '@md/ui'

import PageLoader from '@/components/PageLoader/PageLoader'

/** Full-screen takeover: only for app bootstrap (session, org, tenant resolution). */
export const Loading = () => <PageLoader />

/** In-page loading state, which keeps the layout and sidebar on screen. */
export const InlineLoading = () => (
  <div className="flex w-full justify-center py-16 text-muted-foreground">
    <Spinner size={20} />
  </div>
)
