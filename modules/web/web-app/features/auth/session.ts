import { createStore, useAtom } from 'jotai'
import { atomWithStorage } from 'jotai/utils'

import { LoginResponse } from '@/rpc/api/users/v1/users_pb'

const LS_SESSION_KEY = 'session'

const sessionAtom = atomWithStorage<Session | null>(LS_SESSION_KEY, null, undefined, {
  getOnInit: true,
})

const store = createStore()

export const useSession = () => {
  return useAtom(sessionAtom, { store })
}

export const getSessionToken = (): undefined | string => {
  try {
    const item = store.get(sessionAtom)
    return item?.token
  } catch (e) {
    return undefined
  }
}

export type Session = LoginResponse
