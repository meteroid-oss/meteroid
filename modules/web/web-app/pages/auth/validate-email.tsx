import { useSearchParams } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { ValidateEmailForm } from '@/features/auth/components/ValidateEmailForm'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import type { FunctionComponent } from 'react'


export const ValidateEmail: FunctionComponent = () => {

  const { data, isLoading } = useQuery(getInstance, undefined, {
    refetchOnMount: 'always',
  })

  const [searchParams] = useSearchParams()

  const token = searchParams.get('token')

  if (isLoading) {
    return <Loader />
  }

  if (data && !data.skipEmailValidation && !token) {
    return <div>A validation token is required. Please check your emails.</div>
  }



  return (
    <>
      <div className="font-medium text-xl -mb-0.5">{data?.skipEmailValidation ? "Signup" : "Email address verified!"}</div>
      <div className="text-muted-foreground text-[13px] mb-1 leading-[18px]">
        Set your password for Meteroid to continue.
      </div>
      <ValidateEmailForm />
    </>
  )
}
