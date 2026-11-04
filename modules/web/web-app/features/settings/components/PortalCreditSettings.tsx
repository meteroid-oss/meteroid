import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Form, Switch } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'

import { Slot } from '@/edition'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { UnsavedChangesBar } from '@/features/settings/components/UnsavedChangesBar'
import {
  PortalCreditValues,
  portalCreditSchema,
  portalCreditUpdate,
  portalCreditValues,
} from '@/features/settings/components/portalCreditForm'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useZodForm } from '@/hooks/useZodForm'
import {
  getInvoicingEntity,
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

/** The invoicing entity's switches for what its customers do with their credit in the portal. */
export const PortalCreditSettings = () => {
  const queryClient = useQueryClient()
  const { currentEntity } = useInvoicingEntity()
  const methods = useZodForm({ schema: portalCreditSchema })
  const values = useWatch({ control: methods.control })

  const { reset } = methods
  // Reset on another entity or new stored values only: a refetch that returns the same values
  // under a new object must keep the operator's edits.
  const entityId = currentEntity?.id
  const topUp = currentEntity?.portalTopUpEnabled
  const autoTopUp = currentEntity?.portalAutoTopUpEnabled
  const budgets = currentEntity?.portalBudgetsEnabled
  useEffect(() => {
    reset(
      entityId === undefined
        ? undefined
        : portalCreditValues({
            id: entityId,
            portalTopUpEnabled: topUp ?? false,
            portalAutoTopUpEnabled: autoTopUp ?? false,
            portalBudgetsEnabled: budgets ?? false,
          })
    )
  }, [entityId, topUp, autoTopUp, budgets, reset])

  const updateMut = useMutation(updateInvoicingEntity, {
    onSuccess: res => {
      if (!res.entity) return
      reset(portalCreditValues(res.entity))
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listInvoicingEntities, cardinality: undefined }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoicingEntity,
          input: { id: res.entity.id },
          cardinality: 'finite',
        }),
      })
      toast.success('Customer portal settings saved')
    },
  })

  const onSubmit = async (submitted: PortalCreditValues) => {
    if (!currentEntity) return
    await updateMut.mutateAsync({ id: currentEntity.id, data: portalCreditUpdate(submitted) })
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <SettingsSection
          divided={false}
          title="Customer portal"
          description="What customers can do with their credit in the portal."
          action={<InvoicingEntitySelect />}
        >
          <SettingsRowGroup>
            <SettingsRow
              title="Top-ups"
              description="Paying with a new card needs a card provider on this entity."
            >
              <Switch
                checked={values.portalTopUpEnabled ?? false}
                disabled={!currentEntity}
                onCheckedChange={checked =>
                  methods.setValue('portalTopUpEnabled', checked, { shouldDirty: true })
                }
              />
            </SettingsRow>
            <Slot
              name="portalCreditSettings.rows"
              control={methods.control}
              disabled={!currentEntity}
            />
          </SettingsRowGroup>
        </SettingsSection>
        <UnsavedChangesBar
          show={methods.formState.isDirty}
          saving={updateMut.isPending}
          onDiscard={() => methods.reset()}
        />
      </form>
    </Form>
  )
}
