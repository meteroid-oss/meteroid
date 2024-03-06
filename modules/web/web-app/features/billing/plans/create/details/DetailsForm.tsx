import { useMutation } from '@connectrpc/connect-query'
import { ButtonAlt, FormItem, Input, Label, RadioGroup, RadioGroupItem, Textarea } from '@md/ui'
import { customAlphabet } from 'nanoid'
import { FC, useEffect } from 'react'
import { useController, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useProductFamily } from '@/hooks/useProductFamily'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { createPlanSchema } from '@/lib/schemas/plans'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

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
  const methods = useZodForm({
    schema: createPlanSchema,
    defaultValues: {
      planType: 'STANDARD',
    },
  })

  const createPlan = useMutation(createDraftPlan)

  const navigate = useNavigate()

  const onSubmit = async (data: z.infer<typeof createPlanSchema>) => {
    const plan = await createPlan.mutateAsync({
      name: data.planName,
      description: data.description,
      externalId: data.externalId,
      planType: PlanType[data.planType],
      productFamilyExternalId: 'default',
    })
    navigate(`${plan.plan?.plan?.externalId}/onboarding`)
  }

  useEffect(() => {
    console.log(methods.getValues)
  }, [methods])

  return (
    <form onSubmit={methods.handleSubmit(onSubmit)}>
      <section className="space-y-4">
        <div className="space-y-6 pt-2">
          <FormItem
            name="name"
            label="Name"
            layout="horizontal"
            error={methods.formState.errors.planName?.message}
          >
            <Input type="text" placeholder="Plan name" {...methods.register('planName')} />
          </FormItem>
          {/* TODO */}
          <div className="hidden">
            <div className="w-full border-b "></div>
            <FormItem
              name="name"
              label="Description"
              error={methods.formState.errors.description?.message}
              layout="horizontal"
            >
              <Textarea
                placeholder="This plan gives access to ..."
                {...methods.register('description')}
              />
            </FormItem>
            <div className="w-full border-b "></div>
            <FormItem
              name="name"
              label="Code"
              error={methods.formState.errors.externalId?.message}
              layout="horizontal"
              hint={
                <>
                  Use this reference to uniquely identify the plan when&nbsp;
                  <a className="underline" href="#">
                    using the API
                  </a>
                  .
                </>
              }
            >
              <ExternalIdInput methods={methods} />
            </FormItem>
          </div>
          <div className="w-full border-b border-scale-800 "></div>
          <FormItem name="planType" label="Plan type" layout="horizontal">
            <PlanTypeFormItem methods={methods} />
          </FormItem>
        </div>

        <div className="flex justify-end w-full items-center space-x-3">
          <ButtonAlt type="default" onClick={onCancel}>
            Cancel
          </ButtonAlt>
          <ButtonAlt
            type="primary"
            htmlType="submit"
            loading={createPlan.isPending}
            disabled={!methods.formState.isValid}
          >
            {createPlan.isPending ? 'loading' : 'Configure'}
          </ButtonAlt>
        </div>
      </section>
    </form>
  )
}

const ExternalIdInput = ({ methods }: { methods: Methods<typeof createPlanSchema> }) => {
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
      {...methods.register('externalId')}
      className="rounded-r-none border-r-0"
    />
  )
}

const PlanTypeFormItem = ({ methods }: { methods: Methods<typeof createPlanSchema> }) => {
  const { field } = useController({ name: 'planType', control: methods.control })
  return (
    <RadioGroup
      defaultValue="STANDARD"
      name={field.name}
      onValueChange={field.onChange}
      value={field.value}
    >
      <div className="flex items-center space-x-2">
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
      <div className="flex items-center space-x-2">
        <RadioGroupItem value="FREE" id="r1" disabled />
        <Label htmlFor="r1">
          <PlanTypeCard
            title="Free / Freemium"
            desc="Free plans can be subscribed to without payment information."
          />
        </Label>
      </div>

      <div className="flex items-center space-x-2">
        <RadioGroupItem value="CUSTOM" id="r3" disabled />
        <Label htmlFor="r3">
          <PlanTypeCard
            title="Custom"
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
      <div className="text-sm font-medium text-scale-1100">{title}</div>
      <div className="text-xs text-scale-900">{desc}</div>
    </div>
  </>
)
