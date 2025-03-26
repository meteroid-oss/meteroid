import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { Button, Flex, Form, InputFormField } from '@ui/components'
import { z } from 'zod'

export const InviteOnboardingForm = () => {
  const methods = useZodForm({
    schema: schemas.invitation.invitationSchema,
    defaultValues: {},
    mode: 'onSubmit',
  })

  const onSubmit = async (data: z.infer<typeof schemas.invitation.invitationSchema>) =>
    console.log(data)

  return (
    <Flex direction="column" className="w-full h-full gap-2 p-[52px]">
      <Button
        variant="secondary"
        className="text-muted-foreground rounded-2xl w-20 text-xs mt-[0.5px] h-[28px] mb-2 cursor-default"
      >
        Final Step
      </Button>
      <div className="font-medium text-xl -mb-0.5">Invite your team</div>
      <div className="text-muted-foreground text-[13px] mb-5 leading-[18px]">
        Add the email addresses of the team members and invite them to join.
      </div>
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="flex flex-col gap-2 h-full">
          <InputFormField name="email" control={methods.control} placeholder="rachel@company.com" />
          <Button variant="secondary" className="text-muted-foreground w-20 text-xs mt-2">
            Add more
          </Button>
          <Flex direction="column" className="mt-auto gap-3">
            <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
              Get started
            </Button>
            <Button variant="secondary" className="text-muted-foreground">
              Skip this step
            </Button>
          </Flex>
        </form>
      </Form>
    </Flex>
  )
}
