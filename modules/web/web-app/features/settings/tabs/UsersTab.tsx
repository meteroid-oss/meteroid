import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  InputWithIcon,
  Label,
  Modal,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { CopyIcon, MoreVerticalIcon, UserMinusIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { getInvite } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { OrganizationUserRole, UserWithRole } from '@/rpc/api/users/v1/models_pb'
import {
  leaveOrganization,
  listUsers,
  me,
  removeMember,
} from '@/rpc/api/users/v1/users-UsersService_connectquery'

const userRoleMapping: Record<OrganizationUserRole, string> = {
  [OrganizationUserRole.ADMIN]: 'Owner',
  [OrganizationUserRole.MEMBER]: 'Member',
}

export const UsersTab = () => {
  const [inviteVisible, setInviteVisible] = useState(false)
  const [leaveVisible, setLeaveVisible] = useState(false)
  const [memberToRemove, setMemberToRemove] = useState<UserWithRole | null>(null)
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const meData = useQuery(me).data
  const currentUserId = meData?.user?.id

  const usersData = useQuery(listUsers).data?.users

  const { isAdmin, disableLeaveOrg } = useMemo(() => {
    if (!usersData) return { isAdmin: false, disableLeaveOrg: true }
    const currentUser = usersData.find(u => u.id === currentUserId)
    const isAdminRole = currentUser?.role === OrganizationUserRole.ADMIN
    const adminCount = usersData.filter(u => u.role === OrganizationUserRole.ADMIN).length
    return { isAdmin: isAdminRole, disableLeaveOrg: isAdminRole && adminCount <= 1 }
  }, [usersData, currentUserId])

  const removeMemberMut = useMutation(removeMember, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listUsers) })
      toast.success('Member removed')
    },
    onError: () => {
      toast.error('Failed to remove member')
    },
  })

  const leaveOrganizationMut = useMutation(leaveOrganization, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listUsers) })
      navigate('/')
    },
    onError: (err: Error) => {
      toast.error(err.message ?? 'Failed to leave organization')
    },
  })

  const invite = useQuery(getInvite)

  const inviteLink = useMemo(() => {
    if (!invite?.data?.inviteHash) return undefined
    return `${window.location.origin}/invite?token=${invite.data.inviteHash}`
  }, [invite?.data?.inviteHash])

  const columns: ColumnDef<UserWithRole>[] = [
    {
      header: 'Email',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          {row.original.email}
          {row.original.id === currentUserId && (
            <Badge variant="outline" size="sm">
              You
            </Badge>
          )}
        </div>
      ),
    },
    { header: 'Role', accessorFn: user => userRoleMapping[user.role] },
    ...(isAdmin
      ? [
        {
          id: 'actions',
          cell: ({ row }: { row: { original: UserWithRole } }) => {
            if (row.original.id === currentUserId) return null
            return (
              <div className="flex justify-end">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="sm">
                      <MoreVerticalIcon size={16} />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => setMemberToRemove(row.original)}>
                      <UserMinusIcon size={16} className="mr-2" />
                      Remove
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            )
          },
        } satisfies ColumnDef<UserWithRole>,
      ]
      : []),
  ]

  return (
    <Card className="px-8 py-6 space-y-2">
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
            <TooltipContent>
              You cannot leave the organization as you are the only admin.
            </TooltipContent>
          )}
        </Tooltip>

        <Button variant="secondary" onClick={() => setInviteVisible(true)}>
          Invite users
        </Button>
      </div>

      <div className="max-h-screen overflow-y-auto">
        <SimpleTable columns={columns} data={usersData ?? []}/>
      </div>

      <Modal
        visible={inviteVisible}
        onCancel={() => setInviteVisible(false)}
        hideFooter
        header={<>Invite users</>}
      >
        <div className="p-6 space-y-2">
          <Label className="mb-2 text-muted-foreground">
            Send this invite link to your colleagues
          </Label>
          {inviteLink ? (
            <InputWithIcon
              value={inviteLink}
              readOnly
              icon={<CopyIcon className="group-hover:text-brand"/>}
              className="cursor-pointer"
              containerClassName="group"
              onClick={() => copyToClipboard(inviteLink, () => toast.success('Copied !'))}
            />
          ) : (
            <Skeleton height="2rem" width="100%"/>
          )}
        </div>
      </Modal>

      <Modal
        visible={leaveVisible}
        onCancel={() => setLeaveVisible(false)}
        header={<>Leave organization</>}
        onConfirm={() => {
          setLeaveVisible(false)
          leaveOrganizationMut.mutate({})
        }}
        confirmText="Leave"
      >
        <div className="p-6">
          <p className="text-sm">
            Are you sure you want to leave this organization? You will lose access immediately.
          </p>
        </div>
      </Modal>

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
            Are you sure you want to remove <strong>{memberToRemove?.email}</strong> from this
            organization?
          </p>
        </div>
      </Modal>
    </Card>
  )
}
