import { Login as LoginFeature } from '@/features/auth'
import PageTemplate from '@/features/auth/components/PageTemplate'

export const Login = () => {
  return <PageTemplate form={<LoginFeature />} />
}
