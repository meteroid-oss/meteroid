import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { Loader } from '@/features/auth/components/Loader'
import { acceptInvite, me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

import { INVITE_TOKEN_KEY } from './acceptInvite'

export const InviteAuthenticated = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [inviteToken] = useState(() => sessionStorage.getItem(INVITE_TOKEN_KEY))

  const acceptInviteMut = useMutation(acceptInvite, {
    onSuccess: async data => {
      sessionStorage.removeItem(INVITE_TOKEN_KEY)

      toast.success('Successfully joined organization!', { id: 'invite' })

      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(me) })

      // Force a full page reload to refresh the session and organization list
      if (data.organization?.slug) {
        window.location.href = `/${data.organization.slug}`
      } else {
        window.location.href = '/'
      }
    },
    onError: error => {
      console.error('Error accepting invite:', error)

      sessionStorage.removeItem(INVITE_TOKEN_KEY)

      // If user is already a member, just redirect to home
      if (error.message?.includes('already a member')) {
        toast.info("You're already a member of this organization", { id: 'invite' })
        window.location.href = '/'
      } else {
        toast.error(error.message || 'Failed to accept invite', { id: 'invite' })
        navigate('/')
      }
    },
  })

  useEffect(() => {
    if (!inviteToken) {
      navigate('/')
      return
    }

    acceptInviteMut.mutate({ inviteKey: inviteToken })
  }, [])

  return <Loader />
}
