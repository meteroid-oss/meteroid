import { skipToken } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useEffect, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { useSession } from '@/features/auth/session'
import { useQuery } from '@/lib/connectrpc'
import { getInviteDetails } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'

export const INVITE_TOKEN_KEY = 'pending_invite_token'

export const AcceptInvite = () => {
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const [session] = useSession()
  const [showChoice, setShowChoice] = useState(false)
  const [inviteToken, setInviteToken] = useState<string | null>(null)

  const { data: inviteData, isLoading, isError } = useQuery(
    getInviteDetails,
    inviteToken ? { inviteId: inviteToken } : skipToken,
  )

  useEffect(() => {
    const token = searchParams.get('token') || searchParams.get('invite')

    if (!token) {
      navigate('/')
      return
    }

    sessionStorage.setItem(INVITE_TOKEN_KEY, token)
    setInviteToken(token)

    if (session) {
      navigate('/invite-authenticated')
      return
    }

    setShowChoice(true)
  }, [searchParams, session, navigate])

  if (!showChoice || isLoading) {
    return <Loader />
  }

  if (isError || !inviteData) {
    return (
      <div className="flex flex-col items-center justify-center min-h-screen p-8">
        <div className="max-w-md w-full space-y-4 text-center">
          <h1 className="text-2xl font-semibold">Invalid or expired invite link</h1>
          <p className="text-muted-foreground">
            This invite link is no longer valid. Please contact the organization admin for a new
            invite.
          </p>
        </div>
      </div>
    )
  }

  const roleName = inviteData.role === OrganizationUserRole.ADMIN ? 'Owner' : 'Member'

  return (
    <div className="flex flex-col items-center justify-center min-h-screen p-8">
      <div className="max-w-md w-full space-y-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold mb-2">
            You&apos;ve been invited to join {inviteData.organizationName}!
          </h1>
          <p className="text-muted-foreground mb-6">
            You&apos;ll join as <strong>{roleName}</strong>. Sign in or create an account to accept.
          </p>
        </div>

        <div className="space-y-3">
          <Link to="/login" className="block">
            <Button variant="primary" className="w-full">
              Sign in to existing account
            </Button>
          </Link>
          <Link to="/registration" className="block">
            <Button variant="secondary" className="w-full">
              Create new account
            </Button>
          </Link>
        </div>

        <p className="text-xs text-center text-muted-foreground">
          Your invite will be automatically applied after you sign in or register.
        </p>
      </div>
    </div>
  )
}
