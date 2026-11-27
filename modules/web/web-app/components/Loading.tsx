import PageLoader from '@/components/PageLoader/PageLoader'

/** Full-screen takeover: only for app bootstrap (session, org, tenant resolution). */
export const Loading = () => <PageLoader />

/**
 * In-page loading state, which keeps the layout and sidebar on screen: the outline of a form
 * (a heading, then labelled fields) where it will appear, rather than a spinner. One pulse for
 * the whole block, as a pulse per bar costs style and compositing work.
 */
export const InlineLoading = () => (
  <div
    className="flex w-full animate-pulse flex-col gap-6 py-6"
    aria-busy="true"
    aria-label="Loading"
  >
    <div className="h-5 w-40 rounded-md bg-accent/60" />
    {[0, 1, 2].map(row => (
      <div key={row} className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        {[0, 1].map(col => (
          <div key={col} className="flex flex-col gap-2">
            <div className="h-3 w-24 rounded bg-accent/50" />
            <div className="h-8 w-full rounded-md bg-accent/40" />
          </div>
        ))}
      </div>
    ))}
  </div>
)

/** Rows of a list still loading (a card's table, a menu's items): one bar per row, one pulse. */
export const ListSkeleton = ({ rows = 3, className }: { rows?: number; className?: string }) => (
  <div
    className={`flex animate-pulse flex-col gap-2 ${className ?? 'p-3'}`}
    aria-busy="true"
    aria-label="Loading"
  >
    {Array.from({ length: rows }, (_, row) => (
      <div key={row} className="flex items-center gap-3">
        <div className="h-4 rounded bg-accent/50" style={{ width: `${70 - row * 12}%` }} />
        <div className="ml-auto h-4 w-16 rounded bg-accent/40" />
      </div>
    ))}
  </div>
)
