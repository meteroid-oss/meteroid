import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button, Flex, Form, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { me, onboardMe } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const UserOnboardingForm = () => {
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
    <Flex direction="column" className="w-full h-full gap-2 p-[52px]">
      <div className="font-medium text-xl -mb-0.5">Create your profile</div>
      <div className="text-muted-foreground text-[13px] mb-5 leading-[18px]">
        Let’s get you up and running. This will a minute!
      </div>
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(onSubmit)}
          className="flex flex-col gap-4 h-full w-full"
        >
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-2 w-full">
            <div className="flex-1">
              <InputFormField
                name="firstName"
                label="First name"
                control={methods.control}
                placeholder="John"
                className="w-full"
              />
            </div>
            <div className="flex-1">
              <InputFormField
                name="lastName"
                label="Last name"
                control={methods.control}
                placeholder="Doe"
                className="w-full"
              />
            </div>
          </div>
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

          <Button
            variant="primary"
            type="submit"
            disabled={!methods.formState.isValid}
            className=""
          >
            Continue
          </Button>
        </form>
      </Form>
    </Flex>
  )
}
