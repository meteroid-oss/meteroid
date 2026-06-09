import { createConnectQueryKey, disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  Label,
  Modal,
  Separator,
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
import { CopyIcon, MoreVerticalIcon, RefreshCwIcon, UserMinusIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
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

const inviteSchema = z.object({
  email: z.string().email('Please enter a valid email address'),
})

const userRoleMapping: Record<OrganizationUserRole, string> = {
  [OrganizationUserRole.ADMIN]: 'Owner',
  [OrganizationUserRole.MEMBER]: 'Member',
}

export const UsersTab = () => {
  const [inviteVisible, setInviteVisible] = useState(false)
  const [inviteRole, setInviteRole] = useState<OrganizationUserRole>(OrganizationUserRole.MEMBER)
  const inviteForm = useZodForm({ schema: inviteSchema })
  const [leaveVisible, setLeaveVisible] = useState(false)
  const [memberToRemove, setMemberToRemove] = useState<UserWithRole | null>(null)
  const [inviteToRevoke, setInviteToRevoke] = useState<OrganizationInvite | null>(null)
  const queryClient = useQueryClient()

  const meData = useQuery(me).data
  const currentUserId = meData?.user?.id
  const { data: usersQueryData, refetch: refetchUsers, isFetching: isFetchingUsers } = useQuery(listUsers)
  const usersData = usersQueryData?.users
  const isAdmin = usersData?.find(u => u.id === currentUserId)?.role === OrganizationUserRole.ADMIN
  const { data: pendingInvitesQueryData, refetch: refetchInvites, isFetching: isFetchingInvites } = useQuery(listPendingInvites, isAdmin ? undefined : disableQuery)
  const pendingInvitesData = pendingInvitesQueryData?.invites
  const isRefreshing = isFetchingUsers || isFetchingInvites
  const handleRefresh = () => { refetchUsers(); if (isAdmin) refetchInvites() }
  const adminCount = usersData?.filter(u => u.role === OrganizationUserRole.ADMIN).length ?? 0
  const disableLeaveOrg = isAdmin && adminCount <= 1

  const inviteMemberMut = useMutation(inviteMember, {
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listPendingInvites) })
      void queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listUsers) })
      setInviteVisible(false)
      inviteForm.reset()
      setInviteRole(OrganizationUserRole.MEMBER)
      toast.success('Invite sent')
    },
    onError: (err: Error) => {
      toast.error(err.message ?? 'Failed to send invite')
    },
  })

  const resendInviteMut = useMutation(resendInvite, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listPendingInvites) })
      toast.success('Invite resent')
    },
    onError: () => toast.error('Failed to resend invite'),
  })

  const revokeInviteMut = useMutation(revokeInvite, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listPendingInvites) })
      setInviteToRevoke(null)
      toast.success('Invite revoked')
    },
    onError: () => toast.error('Failed to revoke invite'),
  })

  const removeMemberMut = useMutation(removeMember, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listUsers) })
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
    { header: 'Role', accessorFn: user => userRoleMapping[user.role] },
    ...(isAdmin
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
    { header: 'Role', accessorFn: inv => userRoleMapping[inv.role] },
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
    ...(isAdmin
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
                  `${window.location.origin}/invite?token=${row.original.id}`,
                  () => toast.success('Invite link copied')
                )}>
                  <CopyIcon size={16} className="mr-2"/>Copy invite link
                </DropdownMenuItem>
                {!row.original.isExpired && (
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
    <Card className="px-8 py-6 space-y-6">
      <div className="flex justify-end items-center gap-2">
        <Tooltip>
          <TooltipTrigger asChild>
            <span>
              <Button variant="destructive" onClick={() => setLeaveVisible(true)} disabled={disableLeaveOrg}>
                Leave organization
              </Button>
            </span>
          </TooltipTrigger>
          {disableLeaveOrg && (
            <TooltipContent>You cannot leave as the only admin.</TooltipContent>
          )}
        </Tooltip>
        {isAdmin && (
          <Button variant="secondary" onClick={() => setInviteVisible(true)}>
            Invite member
          </Button>
        )}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="outline" size="sm" disabled={isRefreshing} onClick={handleRefresh}>
              <RefreshCwIcon size={14} className={isRefreshing ? 'animate-spin' : ''} />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Refresh</TooltipContent>
        </Tooltip>
      </div>

      <div className="space-y-2">
        <Label className="text-sm font-medium">Members</Label>
        <SimpleTable columns={memberColumns} data={usersData ?? []}/>
      </div>

      {isAdmin && (
        <>
          <Separator/>
          <div className="space-y-2">
            <Label className="text-sm font-medium">Pending invites</Label>
            {pendingInvitesData === undefined ? (
              <Skeleton height="4rem" width="100%"/>
            ) : pendingInvitesData.length === 0 ? (
              <p className="text-sm text-muted-foreground">No pending invites.</p>
            ) : (
              <SimpleTable columns={inviteColumns} data={pendingInvitesData}/>
            )}
          </div>
        </>
      )}

      {/* Invite modal */}
      <Modal
        visible={inviteVisible}
        onCancel={() => {
          setInviteVisible(false)
          inviteForm.reset()
          setInviteRole(OrganizationUserRole.MEMBER)
        }}
        header={<>Invite member</>}
        onConfirm={() =>
          inviteForm.handleSubmit(({ email }) =>
            inviteMemberMut.mutate({ email, role: inviteRole })
          )()
        }
        confirmText="Send invite"
      >
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
              <Select
                value={String(inviteRole)}
                onValueChange={v => setInviteRole(Number(v) as OrganizationUserRole)}
              >
                <SelectTrigger><SelectValue/></SelectTrigger>
                <SelectContent>
                  <SelectItem value={String(OrganizationUserRole.MEMBER)}>Member</SelectItem>
                  <SelectItem value={String(OrganizationUserRole.ADMIN)}>Owner</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </Form>
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

      {/* Leave org modal */}
      <Modal
        visible={leaveVisible}
        onCancel={() => setLeaveVisible(false)}
        header={<>Leave organization</>}
        onConfirm={() => {
          setLeaveVisible(false);
          leaveOrganizationMut.mutate({})
        }}
        confirmText="Leave"
      >
        <div className="p-6">
          <p className="text-sm">Are you sure you want to leave this organization? You will lose access immediately.</p>
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
    </Card>
  )
}
