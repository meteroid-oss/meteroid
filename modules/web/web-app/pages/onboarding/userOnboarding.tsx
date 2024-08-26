import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button, Form, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Navigate } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import { me, onboardMe } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const UserOnboarding: React.FC = () => {
  const meQuery = useQuery(me)
  if (meQuery.data?.user?.onboarded) {
    if (meQuery.data?.organizations.length >= 1) {
      // this was an invite & the user is new, so he has a single org
      return <Navigate to={`/${meQuery.data.organizations[0].slug}`} />
    } else {
      return <Navigate to="/onboarding/organization" />
    }
  }

  return (
    <>
      <div className="md:w-[500px] w-full  px-6 py-12 sm:px-12 flex flex-col   ">
        <h2 className="text-xl font-semibold">Welcome to Meteroid !</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Let's take a minute to configure your account. <br />
          How should we call you ?
        </p>

        <div className="light h-full pt-4 ">
          <UserOnboardingForm />
        </div>
      </div>
      <div className="grow hidden md:flex overflow:hidden">
        <img
          className="object-cover object-center w-full sm:rounded-lg"
          src="/img/auth.png"
          alt="Onboarding illustration"
        />
      </div>
    </>
  )
}

const UserOnboardingForm = () => {
  const queryClient = useQueryClient()

  const methods = useZodForm({
    schema: schemas.me.accountSchema,
    defaultValues: {},
    mode: 'onSubmit',
  })

  const onboardMeMut = useMutation(onboardMe, {
    onSuccess: async res => {
      if (res.user) {
        queryClient.setQueryData(
          createConnectQueryKey(me),
          createProtobufSafeUpdater(me, prev => {
            return {
              ...prev,
              user: res.user,
            }
          })
        )
      }
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.me.accountSchema>) => {
    await onboardMeMut.mutateAsync({
      department: data.department,
      firstName: data.firstName,
      lastName: data.lastName,
      knowUsFrom: data.knowUsFrom,
    })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="flex flex-col gap-2 h-full">
        <InputFormField
          name="firstName"
          label="First name"
          control={methods.control}
          placeholder="Joe"
        />

        <InputFormField
          name="lastName"
          label="Last name"
          control={methods.control}
          placeholder="Dohn"
        />

        <SelectFormField
          name="department"
          label="Which department do you work in?"
          control={methods.control}
          placeholder="Product, engineering, finance etc"
        >
          <SelectItem value="founder">Founder</SelectItem>
          <SelectItem value="engineering">Engineering</SelectItem>
          <SelectItem value="product">Product</SelectItem>
          <SelectItem value="revenue">Revenue / Finance</SelectItem>
          <SelectItem value="other">Other</SelectItem>
        </SelectFormField>

        <SelectFormField
          name="knowUsFrom"
          label="How did you learn about us?"
          control={methods.control}
          placeholder="Google, Github etc"
        >
          <SelectItem value="github">Github</SelectItem>
          <SelectItem value="search">Search engine</SelectItem>
          <SelectItem value="linkedin">Linkedin</SelectItem>
          <SelectItem value="blog">Blogging platform</SelectItem>
          <SelectItem value="referral">Referral</SelectItem>
          <SelectItem value="other">Other</SelectItem>
        </SelectFormField>

        <div className="flex-grow"></div>

        <Button variant="primary" type="submit" disabled={!methods.formState.isValid} className="">
          Create my account
        </Button>
      </form>
    </Form>
  )
}
