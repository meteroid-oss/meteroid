import { Label } from '@md/ui'
import { toast } from 'sonner'

import { MemberPicker } from '@/features/accounts/components/MemberPicker'
import { useAssignOwner } from '@/features/accounts/hooks/useAssignOwner'
import { useMembers } from '@/features/accounts/hooks/useMembers'
import { usePermissions } from '@/hooks/usePermissions'

/** The durable account owner of a customer; read-only without customers:manage. */
export const OwnerPicker = ({
  customerId,
  customerName,
  ownerUserId,
  className,
  align,
}: {
  customerId: string
  customerName?: string
  ownerUserId?: string
  className?: string
  align?: 'start' | 'end'
}) => {
  const canManage = usePermissions().has('customers:manage')
  const assign = useAssignOwner()
  const { byId } = useMembers()
  const name = customerName || 'the account'

  const onChange = (userId: string | undefined) =>
    assign.mutate(
      { customerIds: [customerId], ownerUserId: userId },
      {
        onSuccess: () =>
          toast.success(
            userId
              ? `${byId.get(userId)?.name ?? 'The member'} now owns ${name}`
              : `Removed the owner of ${name}`
          ),
      }
    )

  return (
    <MemberPicker
      label="Account owner"
      value={ownerUserId}
      placeholder="Unassigned"
      disabled={!canManage || assign.isPending}
      onChange={onChange}
      className={className}
      align={align}
    />
  )
}

/**
 * The owner as a field of the customer forms. Create and update don't carry the owner, so the
 * form assigns it with `useSaveOwner` once the customer is saved.
 */
export const OwnerField = ({
  value,
  onChange,
}: {
  value?: string
  onChange: (userId: string | undefined) => void
}) => (
  <div className="space-y-1">
    <Label>Owner</Label>
    <MemberPicker
      label="Owner"
      value={value}
      placeholder="Unassigned"
      onChange={onChange}
      className="h-9 w-full border border-border px-3"
    />
    <p className="text-xs text-muted-foreground">The member who looks after this account.</p>
  </div>
)

/** Assigns the owner picked in a customer form when it differs from the saved one. */
export const useSaveOwner = () => {
  const assign = useAssignOwner()
  return async (customerId: string, saved: string | undefined, picked: string | undefined) => {
    if (saved === picked) return
    // The customer is saved either way; a failed assignment shows its own error.
    await assign
      .mutateAsync({ customerIds: [customerId], ownerUserId: picked })
      .catch(() => undefined)
  }
}
