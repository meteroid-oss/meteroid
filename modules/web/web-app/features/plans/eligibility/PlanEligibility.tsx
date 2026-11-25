import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'

import { EligibilityEditor } from '@/features/eligibility/EligibilityEditor'
import { fromDraft, toDraft } from '@/features/eligibility/eligibility'
import { ExtraActions } from '@/features/plans/MoreForPlan'
import { usePermissions } from '@/hooks/usePermissions'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  listPlans,
  setPlanEligibility,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import type { Eligibility } from '@/rpc/api/eligibility/v1/models_pb'

/** Who can get the plan. Live: saving never makes a version, and never moves a price. */
export const PlanEligibility = ({
  planId,
  eligibility,
  onDone,
}: {
  planId: string
  eligibility?: Eligibility
  onDone: () => void
}) => {
  const queryClient = useQueryClient()
  const canManage = usePermissions().has('catalog:manage')
  const [draft, setDraft] = useState(() => toDraft(eligibility))
  const dirty = JSON.stringify(fromDraft(draft)) !== JSON.stringify(fromDraft(toDraft(eligibility)))

  const save = useMutation(setPlanEligibility, {
    onSuccess: async () => {
      // The plan, its overview and the plans list all read the plans service.
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listPlans.parent, cardinality: undefined }),
      })
      toast.success('Saved: who can get this plan')
      onDone()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The rule could not be saved')),
  })

  return (
    <div>
      <EligibilityEditor value={draft} onChange={setDraft} disabled={!canManage} />
      {canManage && (
        <ExtraActions>
          <Button type="button" variant="secondary" size="sm" onClick={onDone}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={!dirty || save.isPending}
            onClick={() => save.mutate({ planId, eligibility: fromDraft(draft) })}
          >
            Save
          </Button>
        </ExtraActions>
      )}
    </div>
  )
}
