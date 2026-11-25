import { ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useRef, useState } from 'react'
import { toast } from 'sonner'

import { invalidateLadder } from '@/features/plans/ladder/useAddToPricingPage'
import { setSelfServeLadder } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

/** Saves product lines' ladders whole. `pending` holds the orders being saved, by product line,
 * to show before the server confirms them. */
export const useSaveLadder = () => {
  const queryClient = useQueryClient()
  const { mutateAsync } = useMutation(setSelfServeLadder)
  const [pending, setPending] = useState<Record<string, string[]>>({})
  // One save in flight per product line; moves made meanwhile collapse into the latest order,
  // sent once it lands, so saves can't arrive out of order.
  const inFlight = useRef(new Set<string>())
  const queued = useRef<Record<string, string[]>>({})

  const flush = useCallback(
    async (familyId: string) => {
      inFlight.current.add(familyId)
      try {
        for (let order = queued.current[familyId]; order; order = queued.current[familyId]) {
          delete queued.current[familyId]
          await mutateAsync({ productFamilyLocalId: familyId, planIds: order })
          if (!queued.current[familyId]) await invalidateLadder(queryClient)
        }
      } catch (error) {
        delete queued.current[familyId]
        toast.error(
          (error instanceof ConnectError && error.rawMessage) ||
            'The pricing page order could not be saved'
        )
      } finally {
        inFlight.current.delete(familyId)
        setPending(current => {
          const rest = { ...current }
          delete rest[familyId]
          return rest
        })
      }
    },
    [mutateAsync, queryClient]
  )

  const save = useCallback(
    (familyId: string, order: string[]) => {
      setPending(current => ({ ...current, [familyId]: order }))
      queued.current[familyId] = order
      if (!inFlight.current.has(familyId)) void flush(familyId)
    },
    [flush]
  )

  return { pending, save }
}
