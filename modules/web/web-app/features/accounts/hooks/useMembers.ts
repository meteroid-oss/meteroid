import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listUsers, me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export type Member = {
  id: string
  name: string
  /** The first name, else the email before the @: for tight columns. */
  shortName: string
  /** The full name, or the short one when all there is is an email. */
  displayName: string
  initials: string
  email: string
}

const toMember = (u: {
  id: string
  email: string
  firstName?: string
  lastName?: string
}): Member => {
  const full = [u.firstName, u.lastName].filter(Boolean).join(' ').trim()
  const name = full || u.email
  const initials =
    [u.firstName, u.lastName]
      .map(part => part?.trim()[0])
      .filter(Boolean)
      .join('')
      .toUpperCase() || u.email.slice(0, 2).toUpperCase()
  const shortName = u.firstName?.trim() || u.email.split('@')[0]
  return { id: u.id, name, shortName, displayName: full || shortName, initials, email: u.email }
}

/** Organization members resolved client-side: list rows carry user ids only. */
export const useMembers = () => {
  const users = useQuery(listUsers, {}, { staleTime: 5 * 60_000 })
  const current = useQuery(me, undefined, { staleTime: 5 * 60_000 })

  return useMemo(() => {
    const members = (users.data?.users ?? [])
      .map(toMember)
      .sort((a, b) => a.name.localeCompare(b.name))
    return {
      members,
      byId: new Map(members.map(m => [m.id, m])),
      meId: current.data?.user?.id,
      isLoading: users.isLoading,
    }
  }, [users.data, users.isLoading, current.data])
}
