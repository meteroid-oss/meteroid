import {
  createConnectQueryKey,
  useMutation,
  createProtobufSafeUpdater,
} from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { ButtonAlt } from '@ui/components'
import { atom, useAtom, useSetAtom } from 'jotai'
import { useHydrateAtoms } from 'jotai/utils'
import { focusAtom } from 'jotai-optics'
import { ScopeProvider } from 'jotai-scope'
import { ChevronRightIcon, ChevronDownIcon, PencilIcon, CheckIcon, XIcon } from 'lucide-react'
import { ReactNode, useState } from 'react'
import { DeepPartial } from 'react-hook-form'
import { match } from 'ts-pattern'

import { CapacityForm } from '@/features/billing/plans/pricecomponents/components/CapacityForm'
import { OneTimeForm } from '@/features/billing/plans/pricecomponents/components/OneTimeForm'
import { RecurringForm } from '@/features/billing/plans/pricecomponents/components/RecurringForm'
import { SlotsForm } from '@/features/billing/plans/pricecomponents/components/SlotsForm'
import { SubscriptionRateForm } from '@/features/billing/plans/pricecomponents/components/SubscriptionRateForm'
import { UsageBasedForm } from '@/features/billing/plans/pricecomponents/components/UsageBasedForm'
import {
  addedComponentsAtom,
  editedComponentsAtom,
  feeTypeToHuman,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { mapFeeType } from '@/lib/mapping/feesToGrpc'
import { formPriceCompoentSchema, FormPriceComponent, PriceComponent } from '@/lib/schemas/plans'
import {
  createPriceComponent as createPriceComponentMutation,
  editPriceComponent as editPriceComponentMutation,
  listPriceComponents as listPriceComponentsQuery,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

interface CreatePriceComponentProps {
  createRef: string
  component: DeepPartial<PriceComponent>
}
export const CreatePriceComponent = ({ createRef, component }: CreatePriceComponentProps) => {
  const setAddedComponents = useSetAtom(addedComponentsAtom)

  const overview = usePlanOverview()

  const queryClient = useQueryClient()

  const createPriceComponent = useMutation(createPriceComponentMutation, {
    onSuccess: data => {
      if (!overview?.planVersionId) return
      setAddedComponents(components => components.filter(comp => comp.ref !== createRef))

      if (data.component) {
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponentsQuery, {
            planVersionId: overview.planVersionId,
          }),
          createProtobufSafeUpdater(listPriceComponentsQuery, prev => ({
            components: [...(prev?.components ?? []), data.component!],
          }))
        )
      }
    },
  })

  const cancel = () => {
    // TODO confirm
    setAddedComponents(components => components.filter(comp => comp.ref !== createRef))
  }

  const onSubmit = (data: FormPriceComponent) => {
    const validated = formPriceCompoentSchema.safeParse(data)

    console.log('validated', validated)
    if (!overview?.planVersionId) return

    createPriceComponent.mutate({
      planVersionId: overview.planVersionId,
      name: data.name,
      feeType: mapFeeType(data.fee),
    })
  }

  return (
    <ProviderWrapper init={component}>
      <PriceComponentForm cancel={cancel} onSubmit={onSubmit} />
    </ProviderWrapper>
  )
}

interface EditPriceComponentProps {
  component: PriceComponent
}
export const EditPriceComponent = ({ component }: EditPriceComponentProps) => {
  const setEditedComponents = useSetAtom(editedComponentsAtom)

  const overview = usePlanOverview()

  const queryClient = useQueryClient()

  const editPriceComponent = useMutation(editPriceComponentMutation, {
    onSuccess: data => {
      if (!overview?.planVersionId) return
      setEditedComponents(components => components.filter(compId => compId !== component.id))

      if (data.component) {
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponentsQuery, {
            planVersionId: overview.planVersionId,
          }),
          createProtobufSafeUpdater(listPriceComponentsQuery, prev => {
            const idx = prev?.components?.findIndex(comp => comp.id === component.id) ?? -1
            if (idx === -1 || !data.component) return prev
            // now we update the componet it idx with the new data
            const updated = [...(prev?.components ?? [])]
            updated[idx] = data.component

            return {
              components: updated,
            }
          })
        )
      }
    },
  })

  const cancel = () => {
    // TODO confirm
    setEditedComponents(components => components.filter(comp => comp !== component.id))
  }

  const onSubmit = (data: FormPriceComponent) => {
    if (!overview?.planVersionId) return
    editPriceComponent.mutate({
      planVersionId: overview.planVersionId,
      component: {
        id: component.id,
        feeType: mapFeeType(data.fee),
        name: data.name,
        productItem: undefined, // TODO
      },
    })
  }

  return (
    <ProviderWrapper init={component}>
      <PriceComponentForm cancel={cancel} onSubmit={onSubmit} />
    </ProviderWrapper>
  )
}

const ProviderWrapper = ({
  children,
  init,
}: {
  children: ReactNode
  init: DeepPartial<PriceComponent>
}) => {
  return (
    <ScopeProvider atoms={[editedComponentAtom]}>
      <HydrateAtoms initialValues={init}>{children}</HydrateAtoms>
    </ScopeProvider>
  )
}

export interface FeeFormProps {
  cancel: () => void
  onSubmit: (data: FormPriceComponent['fee']['data']) => void
}

interface PriceComponentFormProps {
  cancel: () => void
  onSubmit: (data: FormPriceComponent) => void
}
const PriceComponentForm = ({ cancel, onSubmit: _onSubmit }: PriceComponentFormProps) => {
  const [feeType] = useAtom(componentFeeTypeAtom)
  const [name] = useAtom(componentNameAtom)

  const onSubmit = (data: FormPriceComponent['fee']['data']) => {
    _onSubmit({ fee: { fee: feeType!, data } as FormPriceComponent['fee'], name: name! })
  }

  return match<typeof feeType, ReactNode>(feeType)
    .with('rate', () => <SubscriptionRateForm cancel={cancel} onSubmit={onSubmit} />)
    .with('slot_based', () => <SlotsForm cancel={cancel} onSubmit={onSubmit} />)
    .with('capacity', () => <CapacityForm cancel={cancel} onSubmit={onSubmit} />)
    .with('usage_based', () => <UsageBasedForm cancel={cancel} onSubmit={onSubmit} />)
    .with('recurring', () => <RecurringForm cancel={cancel} onSubmit={onSubmit} />)
    .with('one_time', () => <OneTimeForm cancel={cancel} onSubmit={onSubmit} />)
    .otherwise(() => <div>Unknown fee type. Please contact support</div>)
}

const editedComponentAtom = atom<DeepPartial<PriceComponent>>({})

const componentNameAtom = focusAtom(editedComponentAtom, optic => optic.prop('name'))
export const componentFeeAtom = focusAtom(editedComponentAtom, optic => optic.prop('fee'))
const componentFeeTypeAtom = focusAtom(componentFeeAtom, optic => optic.optional().prop('fee'))

const HydrateAtoms = ({
  initialValues,
  children,
}: {
  initialValues: DeepPartial<PriceComponent>
  children: ReactNode
}) => {
  useHydrateAtoms([[editedComponentAtom, initialValues]])
  return children
}

export interface EditPriceComponentCard {
  cancel: () => void
  submit: () => void
  children: ReactNode
}
export const EditPriceComponentCard = ({ cancel, submit, children }: EditPriceComponentCard) => {
  const [isCollapsed, setIsCollapsed] = useState(false)
  const [feeType] = useAtom(componentFeeTypeAtom)

  return (
    <div className="flex flex-col grow px-4 py-4 bg-slate-100 border border-slate-400  shadow-md rounded-lg max-w-4xl">
      <div className="flex flex-row justify-between">
        <div className="mt-0.5 flex flex-row items-center ">
          <div
            className="mr-2 cursor-pointer select-none"
            onClick={() => setIsCollapsed(!isCollapsed)}
          >
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 l-5 text-accent-1 group-hover:text-slate-1000" />
            ) : (
              <ChevronDownIcon className="w-5 l-5 text-accent-1 group-hover:text-slate-1000" />
            )}
          </div>
          <div className="flex items-center gap-2 ">
            <EditableComponentName />
            <span className="text-sm pl-4 text-slate-1000">
              {feeType && <>({feeTypeToHuman(feeType)})</>}
            </span>
          </div>
        </div>
        <div className="flex flex-row items-center">
          <ButtonAlt
            type="danger"
            className="font-bold py-1.5 !rounded-r-none bg-transparent"
            onClick={cancel}
          >
            <XIcon size={16} strokeWidth={2} />
          </ButtonAlt>
          <ButtonAlt type="link" className="font-bold py-1.5 !rounded-l-none" onClick={submit}>
            <CheckIcon size={16} strokeWidth={2} />
          </ButtonAlt>
        </div>
      </div>
      <div className="flex flex-col grow px-7">
        <div className="mt-6 flex flex-col grow aria-hidden:hidden" aria-hidden={isCollapsed}>
          {children}
        </div>
      </div>
    </div>
  )
}

const EditableComponentName = () => {
  const [isEditing, setIsEditing] = useState(false)
  const [name, setName] = useAtom(componentNameAtom)

  return (
    <div className="flex flex-row items-center">
      {isEditing ? (
        <input
          className="py-1 px-1 text-base block w-full shadow-sm rounded-md ml-1 border-slate-300"
          value={name}
          autoFocus
          onChange={e => setName(e.target.value)}
          onBlur={() => setIsEditing(false)}
          onKeyUp={e => e.key === 'Enter' && setIsEditing(false)}
        />
      ) : (
        <h4
          className="text-base text-accent-1 font-semibold flex space-x-2 items-center"
          onClick={() => setIsEditing(true)}
        >
          <span>{name}</span>
          <PencilIcon size={12} strokeWidth={2} />
        </h4>
      )}
    </div>
  )
}

// const _renderGraduatedPrice = ({
//   price,
//   config,
//   onPriceChange,
//   showValidation,
// }: {
//   price: PriceWithPhaseOrder
//   config: GraduatedPriceModelConfig
//   onPriceChange: (price: PriceWithPhaseOrder) => void
//   showValidation?: boolean
// }) => {
//   const addTier = () => {
//     const tiers = [...config.tiers]
//     const penultimateLastUnit = parseFloat(tiers[tiers.length - 2].last_unit || '0')
//     tiers.splice(-1, 0, {
//       first_unit: penultimateLastUnit.toString(),
//       last_unit: (penultimateLastUnit + 1).toString(),
//       unit_amount: '',
//     })
//     if (parseFloat(tiers[tiers.length - 1].first_unit) !== penultimateLastUnit + 2) {
//       tiers[tiers.length - 1].first_unit = (penultimateLastUnit + 1).toString()
//     }
//     onPriceChange({
//       ...price,
//       config: {
//         ...config,
//         tiers,
//       },
//     })
//   }

//   const removeTier = (idx: number) => {
//     const tiers = [...config.tiers.filter(tier => tier !== config.tiers[idx])]
//     onPriceChange({
//       ...price,
//       config: {
//         ...config,
//         tiers,
//       },
//     })
//   }

//   return _renderFormInput(
//     'Tier pricing structure',
//     <InputTable
//       overrideHasInteracted={showValidation}
//       headers={['First unit', 'Last unit', 'Per unit']}
//       unremovableIndices={[0, config.tiers.length - 1]}
//       tiers={config.tiers.map((tier, idx) => {
//         const onFirstUnitChange = (firstUnit: string, idx: number) => {
//           if (firstUnit === '' || POSITIVE_NUMERIC_REGEX.test(firstUnit)) {
//             const updatedTiers = [...config.tiers]
//             updatedTiers[idx].first_unit = firstUnit
//             const lastUnit = updatedTiers[idx].last_unit
//             if (lastUnit !== null && parseFloat(lastUnit) <= parseFloat(firstUnit)) {
//               onLastUnitChange((parseFloat(firstUnit) + 1).toString(), idx)
//             }
//             onPriceChange({
//               ...price,
//               config: {
//                 ...config,
//                 tiers: updatedTiers,
//               },
//             })
//           }
//         }

//         const onLastUnitChange = (lastUnit: string, idx: number) => {
//           if (lastUnit === '' || POSITIVE_NUMERIC_REGEX.test(lastUnit)) {
//             const updatedTiers = [...config.tiers]
//             updatedTiers[idx].last_unit = lastUnit
//             if (
//               updatedTiers.length > idx + 1 &&
//               parseFloat(updatedTiers[idx + 1].first_unit) <= parseFloat(lastUnit)
//             ) {
//               onFirstUnitChange(parseFloat(lastUnit).toString(), idx + 1)
//             }
//             onPriceChange({
//               ...price,
//               config: {
//                 ...config,
//                 tiers: updatedTiers,
//               },
//             })
//           }
//         }

//         const onUnitAmountChange = (unitAmount: string) => {
//           if (unitAmount === '' || POSITIVE_NUMERIC_REGEX.test(unitAmount)) {
//             const updatedTiers = [...config.tiers]
//             updatedTiers[idx].unit_amount = unitAmount
//             onPriceChange({
//               ...price,
//               config: {
//                 ...config,
//                 tiers: updatedTiers,
//               },
//             })
//           }
//         }
//         const values: InputTableCellProps[] = [
//           {
//             value: tier.first_unit,
//             onChange: (value: string) => onFirstUnitChange(value, idx),
//             type: 'text',
//             disabled: idx === 0,
//             errorMessage:
//               (idx > 0 &&
//                 config.tiers[idx - 1].last_unit !== null &&
//                 parseFloat(tier.first_unit) !==
//                   parseFloat(config.tiers[idx - 1].last_unit || '0')) ||
//               tier.first_unit === ''
//                 ? ''
//                 : null,
//           },
//           {
//             value:
//               idx !== config.tiers.length - 1 && tier.last_unit !== null ? tier.last_unit : '∞',
//             onChange: (value: string) => onLastUnitChange(value, idx),
//             type: 'text',
//             disabled: idx === config.tiers.length - 1,
//             errorMessage:
//               idx !== config.tiers.length - 1 &&
//               ((tier.last_unit && parseFloat(tier.first_unit) >= parseFloat(tier.last_unit)) ||
//                 tier.last_unit === '')
//                 ? ''
//                 : null,
//           },
//           {
//             value: tier.unit_amount,
//             pricingUnit: price.pricing_unit,
//             onChange: onUnitAmountChange,
//             type: 'price',
//             disabled: false,
//             errorMessage: tier.unit_amount === '' ? '' : null,
//           },
//         ]
//         return values
//       })}
//       addTier={addTier}
//       removeTier={removeTier}
//     />
//   )
// }
