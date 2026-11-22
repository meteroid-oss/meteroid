import { create } from "@bufbuild/protobuf";
import { useEffect } from "react";
import { useLocation, useNavigate } from 'react-router'

import { Loading } from "@/components/Loading";
import { useSession } from "@/features/auth";
import { readRedirectToken } from "@/features/auth/redirectToken";
import { LoginResponseSchema } from "@/rpc/api/users/v1/users_pb";

export const OauthSuccess = () => {

  const location = useLocation()
  const navigate = useNavigate()

  const token = readRedirectToken(location)

  const [, setSession] = useSession()

  useEffect(() => {
    if (token) {
      setSession(create(LoginResponseSchema, { token: token }))
      // Small delay to ensure session is persisted before navigation
      setTimeout(() => {
        navigate('/', { replace: true })
      }, 50)
    }
  }, [token, navigate, setSession])

  return <Loading/>
}
