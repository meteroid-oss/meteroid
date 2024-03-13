import { ColumnDef } from '@tanstack/react-table'
import { Button, Card, Input, InputWithIcon, Label, Modal, Skeleton, Switch } from '@md/ui'
import { useMemo, useState } from 'react'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import { getInvite } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { User, UserRole } from '@/rpc/api/users/v1/models_pb'
import { listUsers } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { CopyIcon } from 'lucide-react'
import { copyToClipboard } from '@/lib/helpers'
import { toast } from 'sonner'

const userRoleMapping: Record<UserRole, string> = {
  [UserRole.ADMIN]: 'Owner',
  [UserRole.MEMBER]: 'Member',
}

export const UsersTab = () => {
  const [visible, setVisible] = useState(false)

  const users = useQuery(listUsers).data?.users ?? []

  const columns: ColumnDef<User>[] = [
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
              icon={<CopyIcon className="group-hover:text-primary" />}
              className="cursor-pointer"
              containerClassName="group"
              onClick={() => copyToClipboard(inviteLink, () => toast.success('Copied !'))}
            />
          ) : (
            <Skeleton height={'2rem'} width="100%" />
          )}
        </div>
      </Modal>
    </Card>
  )
}
