import { useTypedParams } from '@/utils/params'

export const useOrganizationSlug = () => {
  const { organizationSlug } = useTypedParams<{ organizationSlug: string }>()

  return organizationSlug
}
