import { Button, Skeleton } from '@md/ui'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listEntityActivity } from '@/rpc/api/activity/v1/activity-ActivityService_connectquery'
import { ActivityEntry } from '@/rpc/api/activity/v1/activity_pb'

import { ActivityEntryRow } from './ActivityEntryRow'

type Props = {
  entityType: string
  entityId: string
  /** Page size for the initial fetch and each "Load more" click. */
  limit?: number
  /** Optional empty-state copy override. */
  emptyLabel?: string
}

type LoadedPage = {
  entries: ActivityEntry[]
  cursor?: string
}

export const EntityActivityTimeline = ({
  entityType,
  entityId,
  limit = 5,
  emptyLabel = 'No activity yet',
}: Props) => {
  // Append-only stack: every "Load more" pushes the cursor of the next
  // page; we re-render the concatenated entries from all pages fetched
  // so far. Filter changes (entityType/entityId) reset via key change.
  const [pages, setPages] = useState<LoadedPage[]>([])
  const nextCursor = pages.at(-1)?.cursor

  const query = useQuery(
    listEntityActivity,
    { entityType, entityId, limit, cursor: nextCursor },
    { enabled: Boolean(entityId), staleTime: 30_000 }
  )

  // First page is the live query; subsequent pages live in `pages`.
  // We only push to `pages` once the user clicks "Load more".
  const liveEntries = query.data?.entries ?? []
  const liveCursor = query.data?.nextCursor

  const allEntries = [...pages.flatMap(p => p.entries), ...liveEntries]

  if (query.isLoading && pages.length === 0) {
    return (
      <div className="space-y-3 py-3">
        <Skeleton height={16} width={240} />
        <Skeleton height={16} width={200} />
        <Skeleton height={16} width={220} />
      </div>
    )
  }

  if (allEntries.length === 0) {
    return <p className="text-sm text-muted-foreground py-3">{emptyLabel}</p>
  }

  return (
    <div>
      {/* Cap the visible height so the timeline doesn't push the rest of the
          page down when there's a lot of activity. Once loaded, additional
          pages stay reachable via scroll. */}
      <ul className="divide-y max-h-[420px] overflow-y-auto pr-1">
        {allEntries.map(entry => (
          <ActivityEntryRow key={entry.id} entry={entry} compact />
        ))}
      </ul>
      {liveCursor && (
        <div className="pt-2">
          <Button
            variant="outline"
            size="sm"
            disabled={query.isFetching}
            onClick={() => setPages(p => [...p, { entries: liveEntries, cursor: liveCursor }])}
          >
            {query.isFetching ? 'Loading…' : 'Load more'}
          </Button>
        </div>
      )}
    </div>
  )
}
