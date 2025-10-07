import { disableQuery } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useEffect, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { useSession } from '@/features/auth/session'
import { useQuery } from '@/lib/connectrpc'
import { getOrganizationByInviteLink } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

export const INVITE_TOKEN_KEY = 'pending_invite_token'

export const AcceptInvite = () => {
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const [session] = useSession()
  const [showChoice, setShowChoice] = useState(false)
  const [inviteToken, setInviteToken] = useState<string | null>(null)

  const { data: orgData, isLoading: isLoadingOrg } = useQuery(
    getOrganizationByInviteLink,
    inviteToken ? { inviteKey: inviteToken } : disableQuery,
    { enabled: !!inviteToken }
  )

  useEffect(() => {
    const token = searchParams.get('token') || searchParams.get('invite')

    if (!token) {
      // No invite token, redirect to home
      navigate('/')
      return
    }

    // Store the invite token in sessionStorage for later use (scoped to this tab)
    sessionStorage.setItem(INVITE_TOKEN_KEY, token)
    setInviteToken(token)

    // If user is already authenticated, accept the invite immediately
    if (session) {
      navigate('/invite-authenticated')
      return
    }

    // User is not authenticated - show choice between login and registration
    setShowChoice(true)
  }, [searchParams, session, navigate])

  if (!showChoice || isLoadingOrg) {
    return <Loader />
  }

  return (
    <div className="flex flex-col items-center justify-center min-h-screen p-8">
      <div className="max-w-md w-full space-y-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold mb-2">
            {orgData?.organizationName
              ? `You've been invited to join ${orgData.organizationName}!`
              : "You've been invited!"}
          </h1>
          <p className="text-muted-foreground mb-6">
            To accept this invite, please sign in or create a new account.
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
