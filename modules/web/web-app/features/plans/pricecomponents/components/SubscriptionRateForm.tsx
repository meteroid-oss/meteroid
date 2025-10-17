import {
  Button,
  Command,
  CommandEmpty,
  CommandItem,
  CommandList,
  Form,
  GenericFormField,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { useAtom } from 'jotai'
import { XIcon } from 'lucide-react'
import { useState } from 'react'
import { useFieldArray } from 'react-hook-form'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { BillingPeriod } from '@/lib/mapping'
import { RateFee, RateFeeSchema, SlotFeeSchema } from '@/lib/schemas/plans'

import { componentFeeAtom } from '../atoms'

import { FeeFormProps } from './shared'

export const SubscriptionRateForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const data = component?.data as RateFee | undefined

  console.log('data', data)

  const methods = useZodForm({
    schema: RateFeeSchema,
    defaultValues: data,
  })

  return (
    <Form {...methods}>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-border">{/* TODO product */}</div>
          <div className="ml-4 col-span-2">
            <TermRateTable methods={methods} currency={currency}/>
          </div>
        </div>
      </EditPriceComponentCard>
    </Form>
  )
}

export const TermRateTable = ({
  methods,
  currency,
}: {
  methods: Methods<typeof RateFeeSchema> | Methods<typeof SlotFeeSchema>
  currency: string
}) => {
  const periods = ['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL'] as BillingPeriod[]
  const [activeTab, setActiveTab] = useState<string | null>(null)

  const control = methods.control as Methods<typeof RateFeeSchema>['control']

  const { fields, remove, insert } = useFieldArray({
    control,
    name: 'rates',
  })

  const selectedPeriods = fields.map(a => a.term)
  const availablePeriods = periods.filter(
    period => !selectedPeriods.includes(period as BillingPeriod)
  )

  const addNewTab = (period: BillingPeriod) => {
    insert(periods.indexOf(period as BillingPeriod), { term: period, price: '0.00' })

    setActiveTab(period)
  }

  const removeTab = (index: number) => {
    remove(index)
    setActiveTab(null)
  }

  if (selectedPeriods.length === 0) {
    return (
      <div>
        <BillingPeriodSelect
          periods={availablePeriods}
          onSelect={addNewTab}
          label="+ Select a billing period"
        />
        {methods.formState.errors.rates && (
          <div className="text-[0.8rem] font-medium text-destructive">
            <p>{String(methods.formState.errors.rates?.message)}</p>
          </div>
        )}
      </div>
    )
  }

  const selectedTab = activeTab || selectedPeriods[0]

  return (
    <div>
      <Tabs value={selectedTab} onValueChange={setActiveTab}>
        <TabsList>
          {fields.map(field => (
            <TabsTrigger key={field.term} value={field.term} className="capitalize">
              {field.term.toLowerCase()}
            </TabsTrigger>
          ))}
          {availablePeriods.length > 0 && (
            <BillingPeriodSelect periods={availablePeriods} onSelect={addNewTab} label="+ Add"/>
          )}
        </TabsList>

        {fields.map((field, index) => (
          <TabsContent key={field.term} value={field.term}>
            <div className="flex flex-col lg:flex-row gap-4 pt-4">
              <div>
                <GenericFormField
                  control={control}
                  name={`rates.${index}.price`}
                  label="Rate"
                  layout="horizontal"
                  labelClassName="px-4 col-span-3"
                  render={({ field }) => (
                    <UncontrolledPriceInput {...field} currency={currency} className="w-[200px]"/>
                  )}
                />
              </div>
              <Button size="sm" variant="destructiveGhost" onClick={() => removeTab(index)}>
                <XIcon size="16"/>
              </Button>
            </div>
          </TabsContent>
        ))}
      </Tabs>
    </div>
  )
}

interface BillingPeriodSelectProps {
  periods: BillingPeriod[]
  onSelect: (p: BillingPeriod) => void
  label: string
}

const BillingPeriodSelect = ({ periods, onSelect, label }: BillingPeriodSelectProps) => {
  const [open, setOpen] = useState(false)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger>
        <Button variant="ghost" className="text-xs   ">
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[150px] p-0">
        <Command>
          <CommandEmpty>No product family found.</CommandEmpty>
          <CommandList>
            {periods
              .sort((a, b) => {
                const order = ['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL']
                return order.indexOf(a) - order.indexOf(b)
              })
              .map(period => (
                <CommandItem
                  key={period}
                  className="capitalize"
                  onSelect={() => {
                    setOpen(false)
                    onSelect(period)
                  }}
                >
                  {period.toLowerCase()}
                </CommandItem>
              ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
