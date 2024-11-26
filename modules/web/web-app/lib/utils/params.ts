import { useParams } from 'react-router-dom'

// utils to standardize the path params & simplify maintenance

export const ParamsSlugs = {
  tenantSlug: ':tenantSlug',
  organizationSlug: ':organizationSlug',
  planLocalId: ':planLocalId',
  couponLocalId: ':couponLocalId',
  planVersion: ':planVersion',
  feeType: ':feeType',
  invoiceId: ':invoiceId',
  customerId: ':customerId',
}
type AvailableParams = Record<keyof typeof ParamsSlugs, string>

type Common<A, B> = Pick<A & B, keyof A & keyof B>

export const useTypedParams = <A extends Partial<AvailableParams>>() =>
  useParams<Common<A, AvailableParams>>()
