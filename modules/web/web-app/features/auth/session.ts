import { createStore, useAtom } from 'jotai'
import { atomWithStorage } from 'jotai/utils'

import { LoginResponse } from '@/rpc/api/users/v1/users_pb'

const LS_SESSION_KEY = 'session'

const sessionAtom = atomWithStorage<Session | null>(LS_SESSION_KEY, null, undefined, {
  getOnInit: true,
})

const store = createStore()

export const useSession = () => {
  const [session, setSession] = useAtom(sessionAtom, { store })

  const setSessionNotNull = (newSession: Session) => {
    if(!newSession) {
      console.error('Session is null, use clearSession instead.')
      return;
    }
    setSession(newSession)
  }

  const clearSession = () => {
    console.log('Clearing session')
    setSession(null)
  }

  return [session, setSessionNotNull, clearSession] as const

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
