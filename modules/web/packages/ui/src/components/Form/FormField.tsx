import { spaces } from '@md/foundation'

import { Flex } from '@ui/components/Flex'

import { Label } from '../Label'

import { Error, Info, StyledCheckboxFormItem } from './FormField.styled'

interface FormItemProps {
  name?: string
  label: string | null
  hint?: string | React.ReactNode
  error?: string
  optional?: boolean
  children: React.ReactNode
  layout?: 'horizontal' | 'vertical'
}

export const FormItem = ({ layout = 'vertical', children, ...props }: FormItemProps) =>
  layout === 'horizontal' ? (
    <FormFieldHorizontal {...props}>{children}</FormFieldHorizontal>
  ) : (
    <FormFieldVertical {...props}>{children}</FormFieldVertical>
  )

const FormFieldVertical = ({
  name,
  label,
  hint,
  children,
  error,
  optional,
}: Omit<FormItemProps, 'layout'>) => (
  <Flex direction="column" gap={spaces.space2}>
    <Flex direction="row" justify="space-between" align="center">
      <Label htmlFor={name} className="text-slate-1100">
        {label} {!optional && <Error>*</Error>}
      </Label>
    </Flex>

    {children}
    {hint && <FormHint>{hint}</FormHint>}
    {error ? <FormError error={error} /> : optional ? <Info>Optional</Info> : null}
  </Flex>
)

// we want FormFieldHorizontal, like FormFieldVertical but with a horizontal layout
const FormFieldHorizontal = ({ name, label, hint, children, error }: FormItemProps) => (
  <div className="text-sm grid gap-2 md:grid md:grid-cols-12">
    <div className="flex flex-col space-y-2 col-span-4">
      <Label htmlFor={name} className="block text-scale-1100 text-sm">
        {label}
      </Label>
    </div>
    <div className="col-span-8 flex flex-col gap-2">
      <div>{children}</div>
      {!error && hint && <FormHint>{hint}</FormHint>}
      {error && <FormError error={error} />}
    </div>
  </div>
)

export const CheckboxFormItem = ({ name, label, hint, children, error }: FormItemProps) => (
  <StyledCheckboxFormItem>
    <div className="flex space-x-2 items-center">
      {children}
      <Label htmlFor={name} className="font-normal cursor-pointer">
        {label}
      </Label>
    </div>
    {!error && hint && <FormHint>{hint}</FormHint>}
    {error && <FormError error={error} />}
  </StyledCheckboxFormItem>
)

export const FormHint = ({ children }: { children: React.ReactNode }) => <Info>{children}</Info>

export const FormError = ({ error }: { error: string | undefined }) => <Error>{error}</Error>
