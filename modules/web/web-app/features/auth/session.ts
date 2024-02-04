import { useAtom } from 'jotai'
import { atomWithStorage } from 'jotai/utils'

import { LoginResponse } from '@/rpc/api/users/v1/users_pb'

const LS_SESSION_KEY = 'session'

export const getSessionToken = (): undefined | string => {
  try {
    const item = localStorage.getItem(LS_SESSION_KEY)
    return item && JSON.parse(item).token
  } catch (e) {
    return undefined
  }
}

export const sessionAtom = atomWithStorage<Session | null>(LS_SESSION_KEY, null, undefined, {
  getOnInit: true,
})

export const useSession = () => {
  return useAtom(sessionAtom)
}

export type Session = LoginResponse
