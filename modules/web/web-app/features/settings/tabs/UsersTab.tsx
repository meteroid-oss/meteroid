import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  Label,
  Modal,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { formatDistanceToNow } from 'date-fns'
import {
  CopyIcon,
  MoreVerticalIcon,
  RefreshCwIcon,
  UserCogIcon,
  UserMinusIcon,
  XIcon,
} from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { SimpleTable } from '@/components/table/SimpleTable'
import { DangerZone } from '@/features/settings/components/DangerZone'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { usePermissions } from '@/hooks/usePermissions'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  assignRole,
  listRoles,
} from '@/rpc/api/rbac/v1/rbac-RbacService_connectquery'
import { OrganizationUserRole, UserWithRole } from '@/rpc/api/users/v1/models_pb'
import {
  inviteMember,
  leaveOrganization,
  listPendingInvites,
  listUsers,
  me,
  removeMember,
  resendInvite,
  revokeInvite,
} from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { OrganizationInvite } from '@/rpc/api/users/v1/users_pb'

const buildInviteUrl = (id: string) => `${window.location.origin}/invite?token=${id}`

const inviteSchema = z.object({
  email: z.string().email('Please enter a valid email address'),
})

const userRoleMapping: Record<OrganizationUserRole, string> = {
  [OrganizationUserRole.ADMIN]: 'Owner',
  [OrganizationUserRole.MEMBER]: 'Member',
}

// A member's role choice is a single select value: the built-in tiers, or
// `custom:<roleId>` for a custom role (which implies the MEMBER tier).
const OWNER_VALUE = 'owner'
const MEMBER_VALUE = 'member'
const CUSTOM_PREFIX = 'custom:'
const customValue = (roleId: string) => `${CUSTOM_PREFIX}${roleId}`

const currentRoleValue = (user: UserWithRole): string =>
  user.role === OrganizationUserRole.ADMIN
    ? OWNER_VALUE
    : user.customRoleId
      ? customValue(user.customRoleId)
      : MEMBER_VALUE

// Turn a role-select value into the {role, customRoleId} an invite/assign expects.
const roleValueToAssignment = (
  value: string
): { role: OrganizationUserRole; customRoleId?: string } =>
  value === OWNER_VALUE
    ? { role: OrganizationUserRole.ADMIN }
    : value === MEMBER_VALUE
      ? { role: OrganizationUserRole.MEMBER }
      : { role: OrganizationUserRole.MEMBER, customRoleId: value.slice(CUSTOM_PREFIX.length) }

export const UsersTab = () => {
  const [inviteVisible, setInviteVisible] = useState(false)
  const [inviteRoleValue, setInviteRoleValue] = useState<string>(MEMBER_VALUE)
  const [inviteUrl, setInviteUrl] = useState<string | null>(null)
  const inviteForm = useZodForm({ schema: inviteSchema })
  const [memberToRemove, setMemberToRemove] = useState<UserWithRole | null>(null)
  const [inviteToRevoke, setInviteToRevoke] = useState<OrganizationInvite | null>(null)
  const queryClient = useQueryClient()

  const mailerEnabled = useQuery(getInstance).data?.mailerEnabled ?? true

  const meData = useQuery(me).data
  const currentUserId = meData?.user?.id
  const { data: usersQueryData, refetch: refetchUsers, isFetching: isFetchingUsers } = useQuery(listUsers)
  const usersData = usersQueryData?.users
  const isAdmin = usersData?.find(u => u.id === currentUserId)?.role === OrganizationUserRole.ADMIN
  // Owner implies members:manage; custom governance roles can hold it without being Owner.
  const canManageMembers = usePermissions().has('members:manage')
  const { data: pendingInvitesQueryData, refetch: refetchInvites, isFetching: isFetchingInvites } = useQuery(listPendingInvites, canManageMembers ? undefined : skipToken)
  const pendingInvitesData = pendingInvitesQueryData?.invites
  const isRefreshing = isFetchingUsers || isFetchingInvites
  const handleRefresh = () => { refetchUsers(); if (canManageMembers) refetchInvites() }
  const adminCount = usersData?.filter(u => u.role === OrganizationUserRole.ADMIN).length ?? 0
  const disableLeaveOrg = isAdmin && adminCount <= 1

  const rbacEnabled = useEntitlement(entitlementCodes.RBAC)
  const { data: rolesData } = useQuery(listRoles, canManageMembers ? undefined : skipToken)
  const customRoles = rolesData?.roles.filter(r => !r.isSystem) ?? []
  const roleNameById = new Map(customRoles.map(r => [r.id, r.name]))

  // A custom role fully replaces the MEMBER tier's permissions, so show its name
  // instead of the tier. Owner/plain Member fall back to the built-in label.
  const renderRoleCell = (role: OrganizationUserRole, customRoleId?: string) => {
    const customName =
      role !== OrganizationUserRole.ADMIN && customRoleId
        ? roleNameById.get(customRoleId)
        : undefined
    return customName
      ? <Badge variant="secondary" size="sm">{customName}</Badge>
      : <span>{userRoleMapping[role]}</span>
  }

  const [memberToAssign, setMemberToAssign] = useState<UserWithRole | null>(null)
  const [assignValue, setAssignValue] = useState<string>(MEMBER_VALUE)
  const isAssignLastOwner =
    memberToAssign?.role === OrganizationUserRole.ADMIN && adminCount <= 1

  const openAssign = (user: UserWithRole) => {
    setAssignValue(currentRoleValue(user))
    setMemberToAssign(user)
  }

  const assignRoleMut = useMutation(assignRole, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listUsers, cardinality: 'finite' }),
      })
      setMemberToAssign(null)
      toast.success('Role updated')
    },
    onError: (err: Error) => toast.error(err.message ?? 'Failed to update role'),
  })

  const submitAssign = () => {
    if (!memberToAssign) return
    assignRoleMut.mutate({ userId: memberToAssign.id, ...roleValueToAssignment(assignValue) })
  }

  const inviteMemberMut = useMutation(inviteMember, {
    onSuccess: data => {
      void queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPendingInvites,
        cardinality: 'finite'
      }) })
      void queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listUsers,
        cardinality: 'finite'
      }) })
      inviteForm.reset()
      setInviteRoleValue(MEMBER_VALUE)
      if (!mailerEnabled) {
        setInviteUrl(buildInviteUrl(data.inviteId))
      } else {
        setInviteVisible(false)
        toast.success('Invite sent')
      }
    },
    onError: (err: Error) => {
      toast.error(err.message ?? 'Failed to send invite')
    },
  })

  const resendInviteMut = useMutation(resendInvite, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPendingInvites,
        cardinality: 'finite'
      }) })
      toast.success('Invite resent')
    },
    onError: () => toast.error('Failed to resend invite'),
  })

  const revokeInviteMut = useMutation(revokeInvite, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPendingInvites,
        cardinality: 'finite'
      }) })
      setInviteToRevoke(null)
      toast.success('Invite revoked')
    },
    onError: () => toast.error('Failed to revoke invite'),
  })

  const removeMemberMut = useMutation(removeMember, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listUsers,
        cardinality: 'finite'
      }) })
      toast.success('Member removed')
    },
    onError: () => toast.error('Failed to remove member'),
  })

  const leaveOrganizationMut = useMutation(leaveOrganization, {
    onSuccess: () => {
      window.location.replace('/')
    },
    onError: (err: Error) => {
      toast.error(err.message ?? 'Failed to leave organization')
    },
  })

  const memberColumns: ColumnDef<UserWithRole>[] = [
    {
      header: 'Email',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          {row.original.email}
          {row.original.id === currentUserId && (
            <Badge variant="outline" size="sm">You</Badge>
          )}
        </div>
      ),
    },
    {
      header: 'Role',
      cell: ({ row }) => renderRoleCell(row.original.role, row.original.customRoleId),
    },
    ...(canManageMembers
      ? [{
        id: 'actions',
        cell: ({ row }: { row: { original: UserWithRole } }) => {
          if (row.original.id === currentUserId) return null
          return (
            <div className="flex justify-end">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="sm"><MoreVerticalIcon size={16}/></Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={() => openAssign(row.original)}>
                    <UserCogIcon size={16} className="mr-2"/>Change role
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => setMemberToRemove(row.original)}>
                    <UserMinusIcon size={16} className="mr-2"/>Remove
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          )
        },
      } satisfies ColumnDef<UserWithRole>]
      : []),
  ]

  const inviteColumns: ColumnDef<OrganizationInvite>[] = [
    { header: 'Email', accessorKey: 'invitedEmail' },
    { header: 'Role', cell: ({ row }) => renderRoleCell(row.original.role, row.original.customRoleId) },
    { header: 'Invited by', accessorKey: 'invitedByEmail' },
    {
      header: 'Expires',
      cell: ({ row }) => {
        const date = new Date(row.original.expiresAt)
        const label = formatDistanceToNow(date, { addSuffix: true })
        return row.original.isExpired
          ? <Badge variant="destructive" size="sm">Expired {label}</Badge>
          : <span className="text-sm text-muted-foreground">{label}</span>
      },
    },
    ...(canManageMembers
      ? [{
        id: 'actions',
        cell: ({ row }: { row: { original: OrganizationInvite } }) => (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm"><MoreVerticalIcon size={16}/></Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => copyToClipboard(
                  buildInviteUrl(row.original.id),
                  () => toast.success('Invite link copied')
                )}>
                  <CopyIcon size={16} className="mr-2"/>Copy invite link
                </DropdownMenuItem>
                {!row.original.isExpired && mailerEnabled && (
                  <DropdownMenuItem onClick={() => resendInviteMut.mutate({ inviteId: row.original.id })}>
                    <RefreshCwIcon size={16} className="mr-2"/>Resend invite
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem onClick={() => setInviteToRevoke(row.original)}>
                  <XIcon size={16} className="mr-2"/>Revoke invite
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ),
      } satisfies ColumnDef<OrganizationInvite>]
      : []),
  ]

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <SettingsSection
        divided={false}
        title={
          <span className="flex items-center gap-2">
            Members
            <Badge variant="outline" size="sm">
              {usersData?.length ?? 0}
            </Badge>
          </span>
        }
        description="People with access to this organization."
        action={
          <div className="flex items-center gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={isRefreshing}
                  onClick={handleRefresh}
                >
                  <RefreshCwIcon size={14} className={isRefreshing ? 'animate-spin' : ''} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Refresh</TooltipContent>
            </Tooltip>
            {canManageMembers && (
              <Button size="sm" onClick={() => setInviteVisible(true)}>
                Invite member
              </Button>
            )}
          </div>
        }
      >
        <div className="overflow-hidden rounded-lg border border-border">
          <SimpleTable columns={memberColumns} data={usersData ?? []} />
        </div>
      </SettingsSection>

      {canManageMembers && (
        <SettingsSection title="Pending invites" description="Invitations waiting to be accepted.">
          {pendingInvitesData === undefined ? (
            <Skeleton height="4rem" width="100%" />
          ) : pendingInvitesData.length === 0 ? (
            <div className="rounded-lg border border-border bg-card px-4 py-6 text-center text-sm text-muted-foreground">
              No pending invites.
            </div>
          ) : (
            <div className="overflow-hidden rounded-lg border border-border">
              <SimpleTable columns={inviteColumns} data={pendingInvitesData} />
            </div>
          )}
        </SettingsSection>
      )}

      <DangerZone
        actions={[
          {
            title: 'Leave organization',
            description: 'You lose access to this organization immediately. This can’t be undone.',
            buttonLabel: 'Leave organization',
            disabledReason: disableLeaveOrg ? 'You cannot leave as the only admin.' : null,
            onConfirm: () => leaveOrganizationMut.mutate({}),
            confirmation: {
              header: 'Leave organization',
              message:
                'Are you sure you want to leave this organization? You will lose access immediately.',
              confirmLabel: 'Leave',
            },
          },
        ]}
      />

      {/* Invite modal */}
      <Modal
        visible={inviteVisible}
        onCancel={() => {
          setInviteVisible(false)
          setInviteUrl(null)
          inviteForm.reset()
          setInviteRoleValue(MEMBER_VALUE)
        }}
        header={<>Invite member</>}
        onConfirm={() => {
          if (inviteUrl) {
            setInviteVisible(false)
            setInviteUrl(null)
          } else {
            inviteForm.handleSubmit(({ email }) =>
              inviteMemberMut.mutate({ email, ...roleValueToAssignment(inviteRoleValue) })
            )()
          }
        }}
        confirmText={inviteUrl ? 'Done' : mailerEnabled ? 'Send invite' : 'Create invite'}
      >
        {inviteUrl ? (
          <div className="p-6 space-y-3">
            <p className="text-sm text-muted-foreground">
              Share this invite link with your teammate:
            </p>
            <CopyToClipboardButton text={inviteUrl} />
          </div>
        ) : (
          <Form {...inviteForm}>
            <div className="p-6 space-y-4">
              <InputFormField
                control={inviteForm.control}
                name="email"
                label="Email address"
                placeholder="colleague@company.com"
                type="email"
              />
              <div className="space-y-1">
                <Label>Role</Label>
                <Select value={inviteRoleValue} onValueChange={setInviteRoleValue}>
                  <SelectTrigger><SelectValue/></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={MEMBER_VALUE}>Member</SelectItem>
                    <SelectItem value={OWNER_VALUE}>Owner</SelectItem>
                    {rbacEnabled && customRoles.map(role => (
                      <SelectItem key={role.id} value={customValue(role.id)}>
                        {role.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </Form>
        )}
      </Modal>

      {/* Revoke invite modal */}
      <Modal
        visible={!!inviteToRevoke}
        onCancel={() => setInviteToRevoke(null)}
        header={<>Revoke invite</>}
        onConfirm={() => {
          if (!inviteToRevoke) return
          revokeInviteMut.mutate({ inviteId: inviteToRevoke.id })
        }}
        confirmText="Revoke"
      >
        <div className="p-6">
          <p className="text-sm">
            Revoke the invite sent to <strong>{inviteToRevoke?.invitedEmail}</strong>? They will no longer be able to
            use this link.
          </p>
        </div>
      </Modal>

      {/* Remove member modal */}
      <Modal
        visible={!!memberToRemove}
        onCancel={() => setMemberToRemove(null)}
        header={<>Remove member</>}
        onConfirm={() => {
          if (!memberToRemove) return
          setMemberToRemove(null)
          removeMemberMut.mutate({ userId: memberToRemove.id })
        }}
        confirmText="Remove"
      >
        <div className="p-6">
          <p className="text-sm">
            Remove <strong>{memberToRemove?.email}</strong> from this organization?
          </p>
        </div>
      </Modal>

      {/* Change role modal */}
      <Modal
        visible={!!memberToAssign}
        onCancel={() => setMemberToAssign(null)}
        header={<>Change role</>}
        onConfirm={submitAssign}
        confirmText="Save"
        confirmDisabled={assignRoleMut.isPending}
        loading={assignRoleMut.isPending}
      >
        <div className="p-6 space-y-4">
          <p className="text-sm text-muted-foreground">
            Set the role for <strong className="text-foreground">{memberToAssign?.email}</strong>.
          </p>
          <div className="space-y-1">
            <Label>Role</Label>
            <Select value={assignValue} onValueChange={setAssignValue}>
              <SelectTrigger><SelectValue/></SelectTrigger>
              <SelectContent>
                <SelectItem value={OWNER_VALUE}>Owner</SelectItem>
                <SelectItem value={MEMBER_VALUE} disabled={isAssignLastOwner}>Member</SelectItem>
                {rbacEnabled && customRoles.map(role => (
                  <SelectItem key={role.id} value={customValue(role.id)} disabled={isAssignLastOwner}>
                    {role.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {isAssignLastOwner && (
            <p className="text-xs text-muted-foreground">
              This is the last owner. Assign another owner before changing this member&apos;s role.
            </p>
          )}
        </div>
      </Modal>
    </div>
  )
}
