import { ColumnDef } from '@tanstack/react-table'
import { ButtonAlt, Input, Label, Modal } from '@ui/components'
import { useMemo, useState } from 'react'

import { SimpleTable } from '@/components/table/SimpleTable'
import { useQuery } from '@/lib/connectrpc'
import { getInvite } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { User, UserRole } from '@/rpc/api/users/v1/models_pb'
import { listUsers } from '@/rpc/api/users/v1/users-UsersService_connectquery'

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
    <div className="max-w-3xl p-3 space-y-3">
      <div className="flex justify-end ">
        <ButtonAlt type="link" onClick={() => setVisible(true)}>
          Invite users
        </ButtonAlt>
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
        <div className="p-8">
          <Label className="mb-2 text-muted-foreground">
            Send this invite link to your colleagues
          </Label>
          <Input readOnly copy={invite.isSuccess} value={inviteLink ?? 'loading...'} />
        </div>
      </Modal>
    </div>
  )
}
