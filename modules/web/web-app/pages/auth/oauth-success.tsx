import { useEffect } from "react";
import { useSearchParams } from 'react-router-dom'

import { Loading } from "@/components/Loading";
import { useSession } from "@/features/auth";
import { LoginResponse } from "@/rpc/api/users/v1/users_pb";

export const OauthSuccess = () => {

  const [searchParams] = useSearchParams()

  const token = searchParams.get('token')

  const [, setSession] = useSession()

  useEffect(() => {
    token && setSession(new LoginResponse({ token: token }))
  }, [token])

  return <Loading/>
}
