import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  FormInput,
  FormTextarea,
  Label,
  RadioGroup,
  RadioGroupItem,
  GenericFormField,
  Input,
  cn,
  Spinner,
} from '@ui2/components'
import { customAlphabet } from 'nanoid'
import { FC, useEffect } from 'react'
import {
  ControllerRenderProps,
  FieldPath,
  FieldValues,
  useController,
  useWatch,
} from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useProductFamily } from '@/hooks/useProductFamily'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { createPlanSchema } from '@/lib/schemas/plans'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useTypedParams } from '@/utils/params'
import { useQueryClient } from '@tanstack/react-query'

const nanoid = customAlphabet('1234567890abcdef', 5)

export const generateFromName = (name: string) => {
  const convert = (separator: string, a: string) => {
    return a
      .split(/_|-|(![A-Z])(?=[A-Z][a-z])|(?![^A-Z_-])(?=[A-Z])|(?![A-Za-z])(?=[^A-Za-z])/g)
      ?.filter(a => a)
      ?.map(a => a.toLowerCase())
      .join(separator)
  }
  const normalized = name.replaceAll(/[^A-Za-z\d_\- ]/g, '')
  const converted = normalized.includes(' ')
    ? normalized
        .split(' ')
        .map(a => convert('_', a))
        .join('-')
    : convert('-', normalized)
  return !converted || converted.length === 0 ? nanoid() : converted
}

interface Props {
  onCancel: () => void
}
export const DetailsForm: FC<Props> = ({ onCancel }) => {
  const { familyExternalId } = useTypedParams()
  const methods = useZodForm({
    schema: createPlanSchema,
    defaultValues: {
      planType: 'STANDARD',
    },
  })
  const queryClient = useQueryClient()

  const createPlan = useMutation(createDraftPlan, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
    },
  })

  const navigate = useNavigate()

  useProductFamily

  const onSubmit = async (data: z.infer<typeof createPlanSchema>) => {
    const plan = await createPlan.mutateAsync({
      name: data.planName,
      description: data.description,
      externalId: data.externalId,
      planType: PlanType[data.planType],
      productFamilyExternalId: familyExternalId,
    })
    navigate(`${plan.plan?.plan?.externalId}/onboarding`)
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <section className="space-y-4">
          <div className="space-y-6 py-2">
            <FormInput
              name="planName"
              label="Name"
              layout="horizontal"
              control={methods.control}
              type="text"
              placeholder="Plan name"
            />
            {/* TODO */}
            <div className="hidden">
              <div className="w-full border-b "></div>
              <FormTextarea
                name="description"
                label="Description"
                control={methods.control}
                placeholder="This plan gives access to ..."
                layout="horizontal"
              />
              <div className="w-full border-b "></div>
              <GenericFormField
                name="externalId"
                label="Code"
                layout="horizontal"
                // hint={ TODO
                //   <>
                //     Use this reference to uniquely identify the plan when&nbsp;
                //     <a className="underline" href="#">
                //       using the API
                //     </a>
                //     .
                //   </>
                // }
                render={({ field, className }) => (
                  <ExternalIdInput methods={methods} field={field} className={className} />
                )}
              />
            </div>
            <div className="w-full border-b border-slate-800 "></div>
            <GenericFormField
              name="planType"
              label="Plan type"
              layout="horizontal"
              render={({ className, field }) => (
                <PlanTypeFormItem methods={methods} field={field} className={className} />
              )}
            />
          </div>

          <div className="flex justify-end w-full items-center space-x-3">
            <Button variant="secondary" onClick={onCancel}>
              Cancel
            </Button>
            <Button variant="alternative" type="submit" disabled={!methods.formState.isValid}>
              {createPlan.isPending ? (
                <>
                  <Spinner /> Loading...
                </>
              ) : (
                'Configure'
              )}
            </Button>
          </div>
        </section>
      </form>
    </Form>
  )
}

const ExternalIdInput = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  methods,
  field,
  className,
}: {
  field: ControllerRenderProps<TFieldValues, TName>
  methods: Methods<typeof createPlanSchema>
  className: string
}) => {
  const planName = useWatch({ control: methods.control, name: 'planName' })
  const { productFamily } = useProductFamily()

  useEffect(() => {
    const generate = () => {
      // we generate a alphanumeric + -_ api name absed on the product family and the the plan name
      const name = methods.getValues('planName')
      const nameCleaned = generateFromName(name)
      const externalId = `${productFamily?.externalId}_${nameCleaned}`
      methods.setValue('externalId', externalId, { shouldValidate: true })
    }

    if (planName && !methods.getFieldState('externalId').isDirty) generate()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [planName])
  return (
    <Input
      type="text"
      placeholder="external_id"
      {...field}
      className={cn('rounded-r-none border-r-0', className)}
    />
  )
}

const PlanTypeFormItem = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  methods,
  className,
}: {
  field: ControllerRenderProps<TFieldValues, TName>
  methods: Methods<typeof createPlanSchema>
  className: string
}) => {
  const { field } = useController({ name: 'planType', control: methods.control })
  return (
    <RadioGroup
      defaultValue="STANDARD"
      name={field.name}
      onValueChange={field.onChange}
      value={field.value}
      className={className}
    >
      <div className="flex items-center space-x-4">
        <RadioGroupItem value="STANDARD" id="r2" />
        <Label htmlFor="r2">
          <PlanTypeCard
            title="Standard"
            desc={
              <>
                Standard plans are meant to be subscribed by your customers in a <b>self-serve</b>{' '}
                manner.
              </>
            }
          />
        </Label>
      </div>
      <div className="flex items-center space-x-4">
        <RadioGroupItem value="FREE" id="r1" disabled />
        <Label htmlFor="r1">
          <PlanTypeCard
            title="Free / Freemium (disabled)"
            desc="Free plans can be subscribed to without payment information."
          />
        </Label>
      </div>

      <div className="flex items-center space-x-4">
        <RadioGroupItem value="CUSTOM" id="r3" disabled />
        <Label htmlFor="r3">
          <PlanTypeCard
            title="Custom (disabled)"
            desc={
              <>
                Custom plans allows to generate quotes and to be extended per customer or customer
                groups. They are meant to be used in <b>sales-led</b> opportunities.
              </>
            }
          />
        </Label>
      </div>
    </RadioGroup>
  )
}

interface PlanTypeCardProps {
  title: string
  desc: React.ReactNode
}
const PlanTypeCard: FC<PlanTypeCardProps> = ({ title, desc }) => (
  <>
    <div className="flex flex-col ">
      <div className="text-sm font-medium text-foreground">{title}</div>
      <div className="text-xs text-muted-foreground">{desc}</div>
    </div>
  </>
)
