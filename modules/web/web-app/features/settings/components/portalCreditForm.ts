import { z } from 'zod'

import type {
  InvoicingEntity,
  InvoicingEntityData,
} from '@/rpc/api/invoicingentities/v1/models_pb'

export const portalCreditSchema = z.object({
  portalTopUpEnabled: z.boolean(),
  portalAutoTopUpEnabled: z.boolean(),
  portalBudgetsEnabled: z.boolean(),
})

export type PortalCreditValues = z.infer<typeof portalCreditSchema>

type StoredPortalCredit = Pick<
  InvoicingEntity,
  'id' | 'portalTopUpEnabled' | 'portalAutoTopUpEnabled' | 'portalBudgetsEnabled'
>

export const portalCreditValues = (entity: StoredPortalCredit | undefined) =>
  entity
    ? {
        portalTopUpEnabled: entity.portalTopUpEnabled,
        portalAutoTopUpEnabled: entity.portalAutoTopUpEnabled,
        portalBudgetsEnabled: entity.portalBudgetsEnabled,
      }
    : undefined

export const portalCreditUpdate = (
  values: PortalCreditValues
): Pick<
  InvoicingEntityData,
  'portalTopUpEnabled' | 'portalAutoTopUpEnabled' | 'portalBudgetsEnabled'
> => ({
  portalTopUpEnabled: values.portalTopUpEnabled,
  portalAutoTopUpEnabled: values.portalAutoTopUpEnabled,
  portalBudgetsEnabled: values.portalBudgetsEnabled,
})
