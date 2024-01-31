import { Registration as RegistrationFeature } from '@/features/auth/Registration'
import PageTemplate from '@/features/auth/components/PageTemplate'

export const Registration = () => {
  return <PageTemplate form={<RegistrationFeature />} />
}
