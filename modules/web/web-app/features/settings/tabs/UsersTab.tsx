import { Button, Card, InputWithIcon, Label, Modal, Skeleton } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { CopyIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { getInvite } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { OrganizationUserRole, UserWithRole } from '@/rpc/api/users/v1/models_pb'
import { listUsers } from '@/rpc/api/users/v1/users-UsersService_connectquery'

const userRoleMapping: Record<OrganizationUserRole, string> = {
  [OrganizationUserRole.ADMIN]: 'Owner',
  [OrganizationUserRole.MEMBER]: 'Member',
}

export const UsersTab = () => {
  const [visible, setVisible] = useState(false)

  const users = useQuery(listUsers).data?.users ?? []

  const columns: ColumnDef<UserWithRole>[] = [
    { header: 'Email', accessorFn: user => user.email },
    { header: 'Role', accessorFn: user => userRoleMapping[user.role] },
  ]

  const invite = useQuery(getInvite)

  const inviteLink = useMemo(() => {
    if (!invite?.data?.inviteHash) {
      return undefined
    }
    return `${window.location.origin}/registration?invite=${invite.data.inviteHash}`
  }, [invite?.data?.inviteHash])

  return (
    <Card className="px-8 py-6">
      <div className="flex justify-end ">
        <Button variant="secondary" onClick={() => setVisible(true)}>
          Invite users
        </Button>
      </div>
      <div className=" max-h-screen overflow-y-auto">
        <SimpleTable columns={columns} data={users} />
      </div>

      <Modal
        visible={visible}
        onCancel={() => setVisible(false)}
        hideFooter
        header={<>Invite users</>}
      >
        <div className="p-6 space-y-2">
          {/* <div className="text-sm pb-4">
            <Switch /> Enable invite link
          </div> */}

          <Label className="mb-2 text-muted-foreground">
            Send this invite link to your colleagues
          </Label>

          {inviteLink ? (
            <InputWithIcon
              value={inviteLink}
              readOnly
              icon={<CopyIcon className="group-hover:text-brand" />}
              className="cursor-pointer"
              containerClassName="group"
              onClick={() => copyToClipboard(inviteLink, () => toast.success('Copied !'))}
            />
          ) : (
            <Skeleton height="2rem" width="100%" />
          )}
        </div>
      </Modal>
    </Card>
  )
}
