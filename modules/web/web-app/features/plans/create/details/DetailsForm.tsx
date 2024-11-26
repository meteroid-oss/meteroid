import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  GenericFormField,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  SelectFormField,
  SelectItem,
  Spinner,
  TextareaFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { customAlphabet } from 'nanoid'
import { FC } from 'react'
import { ControllerRenderProps, FieldPath, FieldValues, useController } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { createPlanSchema } from '@/lib/schemas/plans'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

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
  const familiesQuery = useQuery(listProductFamilies)
  const families = (familiesQuery.data?.productFamilies ?? []).sort((a, b) =>
    a.id > b.id ? 1 : -1
  )
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

  const onSubmit = async (data: z.infer<typeof createPlanSchema>) => {
    const plan = await createPlan.mutateAsync({
      name: data.planName,
      description: data.description,
      planType: PlanType[data.planType],
      productFamilyLocalId: data.productFamilyLocalId,
    })
    if (data.planType === 'FREE') {
      navigate(`../${plan.plan?.plan?.localId}`)
    } else {
      navigate(`../${plan.plan?.plan?.localId}/draft/onboarding`)
    }
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <section className="space-y-4">
          <div className="space-y-6 py-2">
            <InputFormField
              name="planName"
              label="Name"
              layout="horizontal"
              control={methods.control}
              type="text"
              placeholder="Plan name"
            />

            <SelectFormField
              name="productFamilyLocalId"
              label="Product line"
              layout="horizontal"
              placeholder="Select a product line"
              className="max-w-[320px]  "
              empty={families.length === 0}
              control={methods.control}
            >
              {families.map(f => (
                <SelectItem value={f.localId} key={f.localId}>
                  {f.name}
                </SelectItem>
              ))}
            </SelectFormField>

            {/* TODO */}
            <div className="hidden">
              <div className="w-full border-b "></div>

              <TextareaFormField
                name="description"
                label="Description"
                control={methods.control}
                placeholder="This plan gives access to ..."
                layout="horizontal"
              />
            </div>
            <div className="w-full border-b border-border "></div>
            <GenericFormField
              name="planType"
              label="Plan type"
              layout="horizontal"
              control={methods.control}
              render={({ className, field }) => (
                <PlanTypeFormItem methods={methods} field={field} className={className} />
              )}
            />
          </div>

          <div className="flex justify-end w-full items-center space-x-3">
            <Button variant="secondary" onClick={onCancel}>
              Cancel
            </Button>

            <Button variant="primary" type="submit" disabled={!methods.formState.isValid}>
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
      <div className="flex items-center space-x-4 h-14">
        <div>
          <RadioGroupItem value="STANDARD" id="r2" />
        </div>
        <Label htmlFor="r2">
          <PlanTypeCard
            title="Standard"
            desc={
              <>
                Standard plans are meant to be subscribed by your customers in a <b>self-serve</b>{' '}
                or scalable manner.
              </>
            }
          />
        </Label>
      </div>
      <div className="flex items-center space-x-4  h-14">
        <div>
          <RadioGroupItem value="FREE" id="r1" />
        </div>
        <Label htmlFor="r1">
          <PlanTypeCard
            title="Free / Freemium "
            desc="Free plans cannot include paid components, but can give access to some features."
          />
        </Label>
      </div>

      <div className="flex items-center space-x-4  h-14">
        <div>
          <RadioGroupItem value="CUSTOM" id="r3" className="aspect-square h-4 w-4" />
        </div>
        <Label htmlFor="r3">
          <PlanTypeCard
            title="Custom "
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
