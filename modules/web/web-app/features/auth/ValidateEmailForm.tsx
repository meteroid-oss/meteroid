import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { Button, Form, InputFormField } from '@md/ui'
import { z } from 'zod'

export const ValidateEmailForm = () => {
  const methods = useZodForm({
    schema: schemas.me.validateEmailSchema,
    defaultValues: {
      code: '',
    },
  })

  // const registerMut = useMutation(register, {
  //   onSuccess: async res => {
  //     await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })
  //     setSession(res)
  //   },
  //   onError: err => {
  //     methods.setError('email', {
  //       message: err.rawMessage ?? 'An error occurred, please try again later.',
  //     })
  //   },
  // })

  const onSubmit = async (data: z.infer<typeof schemas.me.validateEmailSchema>) => {
    // await registerMut.mutateAsync({
    //   email: data.email,
    //   inviteKey: invite,
    // })
    // navigate('/login')
    console.log(data)
  }
  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="flex flex-col gap-6">
          <InputFormField
            name="code"
            label="Login code"
            control={methods.control}
            placeholder="Enter your code"
          />
          <Button variant="secondary" type="submit" disabled={!methods.formState.isValid}>
            Continue with login code
          </Button>
          <Button variant="ghost">Back to Sign up</Button>
        </div>
      </form>
    </Form>
  )
}
