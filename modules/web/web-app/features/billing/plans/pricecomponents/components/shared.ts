import { FormPriceComponent } from '@/lib/schemas/plans'

export interface FeeFormProps {
  cancel: () => void
  onSubmit: (data: FormPriceComponent['fee']['data']) => void
}
