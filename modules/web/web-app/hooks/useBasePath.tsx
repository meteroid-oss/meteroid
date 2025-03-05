import { useTypedParams } from '@/utils/params'

export const useBasePath = () => {
  const { organizationSlug, tenantSlug } = useTypedParams()

  return `/${organizationSlug}/${tenantSlug}`
}
