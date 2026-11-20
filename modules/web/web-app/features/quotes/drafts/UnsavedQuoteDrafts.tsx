import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Card, CardContent, CardHeader, CardTitle } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { formatDistanceToNow } from 'date-fns'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { useMembers } from '@/features/accounts/hooks/useMembers'
import {
  localDraftKey,
  removeLocalDraft,
  useLocalDraftScope,
} from '@/features/quotes/editor/localDraft'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteQuoteDraft,
  listQuoteDrafts,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseUtc } from '@/utils/date'

/** New quotes someone started and never created, newest first. */
export const UnsavedQuoteDrafts = () => {
  const basePath = useBasePath()
  const scope = useLocalDraftScope()
  const queryClient = useQueryClient()
  const { byId } = useMembers()
  const draftsQuery = useQuery(listQuoteDrafts)

  const discard = useMutation(deleteQuoteDraft, {
    onSuccess: async (_, request) => {
      if (request.id) removeLocalDraft(localDraftKey(scope, { draftId: request.id }))
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listQuoteDrafts, cardinality: undefined }),
      })
    },
    onError: error => toast.error(`Failed to discard the draft: ${error.message}`),
  })

  const drafts = draftsQuery.data?.drafts ?? []
  if (drafts.length === 0) return null

  return (
    <Card>
      <CardHeader className="py-3">
        <CardTitle className="text-sm">Unsaved drafts</CardTitle>
      </CardHeader>
      <CardContent className="divide-y pb-2">
        {drafts.map(draft => {
          const author = draft.updatedBy ? byId.get(draft.updatedBy)?.displayName : undefined
          return (
            <div key={draft.id} className="flex items-center justify-between gap-4 py-2">
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">
                  {draft.label || 'Untitled quote'}
                </div>
                <div className="text-xs text-muted-foreground">
                  Edited {formatDistanceToNow(parseUtc(draft.updatedAt), { addSuffix: true })}
                  {author ? ` by ${author}` : ''}
                </div>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={discard.isPending}
                  onClick={() => discard.mutate({ id: draft.id })}
                >
                  Discard
                </Button>
                <Button size="sm" variant="outline" asChild>
                  <Link to={`${basePath}/quotes/create?draft=${draft.id}`}>Resume</Link>
                </Button>
              </div>
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}
