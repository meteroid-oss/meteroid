import { useEffect } from "react";
import { useSearchParams, useNavigate } from 'react-router-dom'

import { Loading } from "@/components/Loading";
import { useSession } from "@/features/auth";
import { LoginResponse } from "@/rpc/api/users/v1/users_pb";

export const OauthSuccess = () => {

  const [searchParams] = useSearchParams()
  const navigate = useNavigate()

  const token = searchParams.get('token')

  const [, setSession] = useSession()

  useEffect(() => {
    if (token) {
      setSession(new LoginResponse({ token: token }))
      // Small delay to ensure session is persisted before navigation
      setTimeout(() => {
        navigate('/')
      }, 50)
    }
  }, [token, navigate, setSession])

  return <Loading/>
}
