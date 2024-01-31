import { Modal } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { FC, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { ControlledCheckbox } from '@/components/form/ControlledCheckbox'
import { SimpleTable } from '@/components/table/SimpleTable'
import { useBillingPeriods } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { Cadence } from '@/lib/schemas/plans'

export const BillingPeriodModal: FC = () => {
  const onSelectCancel = () => {
    navigate('..')
  }

  const [billingPeriods, setBillingPeriods] = useBillingPeriods()

  const navigate = useNavigate()

  const schema = z.object({
    periods: z.array(z.boolean().optional()),
  })

  const rows = ['Monthly', 'Quarterly', 'Annual'].map(name => ({
    name,
    cadence: name.toUpperCase() as Cadence,
  }))

  const methods = useZodForm({
    schema,
    defaultValues: {
      periods: rows.map(row => (billingPeriods ?? []).includes(row.cadence)),
    },
  })

  const columns = useMemo<ColumnDef<{ name: string }>[]>(
    () => [
      {
        id: 'selection',
        cell: ({ row }) => (
          <span className="w-full flex items-center justify-center">
            <ControlledCheckbox
              {...methods.withControl(`periods.${row.index}`)}
              id={`periods.${row.index}`}
            />
          </span>
        ),
      },
      {
        id: 'Term',
        cell: ({ row }) => <label htmlFor={`periods.${row.index}`}>{row.original.name}</label>,
      },
    ],
    []
  )

  const saveCadence = (cadences: Cadence[]) => {
    setBillingPeriods(cadences)
    navigate('..')
  }

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={
        <>
          <>Billing terms</>
        </>
      }
      size="large"
      onCancel={onSelectCancel}
      onConfirm={() =>
        methods.handleSubmit(
          data =>
            saveCadence(
              rows.filter((_, i) => data.periods[i]).map(row => row.name.toUpperCase() as Cadence)
            ),
          err => console.log(err)
        )()
      }
    >
      <div className="px-6">
        <div className="p-4 text-sm text-slate-1000 flex flex-col">
          <span>Select the terms which will be available for your customers to pick from.</span>
          <span>You can define different prices based on these terms.</span>
        </div>

        <div className="p-4">
          <SimpleTable
            columns={columns}
            data={rows}
            headTrClasses="!hidden"
            containerClassName="max-w-xs"
          />
        </div>
      </div>
    </Modal>
  )
}

interface PricingModelCardProps {
  title: string
  subtitle: string
  hero: React.ReactNode
  action: () => void
}
export const PricingModelCard = (props: PricingModelCardProps) => {
  return (
    <div
      className="p-4 bg-gray-100 border rounded-md hover:border-brand-1000 hover:border-2 cursor-pointer"
      onClick={props.action}
    >
      <h2 className="text-lg font-semibol d">{props.title}</h2>
      <div>{props.hero}</div>
      <div>
        <span>{props.subtitle}</span>
      </div>
    </div>
  )
}
