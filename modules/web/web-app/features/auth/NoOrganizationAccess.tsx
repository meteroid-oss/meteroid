import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Flex,
} from '@ui/components'
import { ChevronRight, Plus } from 'lucide-react'
import { useNavigate } from 'react-router-dom'

import { MeteroidTitle } from '@/components/svg'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

const orgAvatarStyle = {
  background: `linear-gradient(0deg, #C7B3FE, #C7B3FE), linear-gradient(0deg, #B69EF0, #B69EF0)`,
}

interface Props {
  organizationSlug?: string
}

export const NoOrganizationAccess: React.FC<Props> = ({ organizationSlug }) => {
  const navigate = useNavigate()
  const logout = useLogout()

  const meQuery = useQuery(me)
  const getInstanceQuery = useQuery(getInstance)

  const organizations = meQuery.data?.organizations ?? []
  const email = meQuery.data?.user?.email
  const multiOrgEnabled = getInstanceQuery.data?.multiOrganizationEnabled ?? false

  return (
    <div className="min-h-screen w-full flex flex-col bg-background text-foreground">
      <Flex justify="between" align="center" className="p-6">
        <MeteroidTitle />
        <div className="text-xs">
          <span className="text-muted-foreground mr-1">Logged in as {email}</span>
          <span
            className="underline cursor-pointer"
            onClick={() => logout('User clicked on logout')}
          >
            Log out
          </span>
        </div>
      </Flex>

      <Flex justify="center" align="center" className="grow px-4 pb-24">
        <Card className="w-full max-w-md">
          <CardHeader>
            <CardTitle>No access to this organization</CardTitle>
            <CardDescription>
              {organizationSlug ? (
                <>
                  You don&apos;t have access to{' '}
                  <span className="font-medium text-foreground">{organizationSlug}</span>, or it
                  doesn&apos;t exist.
                </>
              ) : (
                <>You don&apos;t have access to this organization, or it doesn&apos;t exist.</>
              )}{' '}
              {organizations.length > 0 && 'Select one of your organizations below.'}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            {organizations.length > 0 ? (
              organizations.map(org => (
                <button
                  key={org.id}
                  onClick={() => {
                    window.location.href = `/${org.slug}`
                  }}
                  className="flex items-center justify-between w-full rounded-md border border-border px-3 py-2.5 text-left hover:bg-accent transition-colors"
                >
                  <Flex align="center" className="gap-2.5 min-w-0">
                    <div className="flex aspect-square h-6 w-6 rounded-md shrink-0" style={orgAvatarStyle} />
                    <span className="truncate text-sm font-medium">{org.tradeName}</span>
                  </Flex>
                  <ChevronRight size={16} className="text-muted-foreground shrink-0" />
                </button>
              ))
            ) : (
              <div className="text-sm text-muted-foreground py-2">
                You are not a member of any organization yet.
              </div>
            )}

            {multiOrgEnabled && (
              <Button
                variant="secondary"
                className="mt-2 w-full"
                onClick={() => navigate('/onboarding/organization')}
              >
                <Plus size={16} className="mr-2" />
                Create a new organization
              </Button>
            )}
          </CardContent>
        </Card>
      </Flex>
    </div>
  )
}
