import { spaces } from '@md/foundation'
import * as React from 'react'

import { Label } from '@ui/components/ui/label'

import { Flex } from './flex'

/**
 * Prefer the new ../form for new components.
 * Note that the new forms require a FormProvider, unlike this one.
 */

const Info = ({ children }: { children: React.ReactNode }) => (
  <span className="text-muted-foreground">{children}</span>
)

const Error = ({ children }: { children: React.ReactNode }) => (
  <span className="text-danger text-sm">{children}</span>
)

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
      <Label htmlFor={name} className="text-muted-foreground">
        {label} {!optional && <span className="text-destructive">*</span>}
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
      <Label htmlFor={name} className="block text-muted-foreground text-sm">
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
  <div className="border border-border rounded-sm">
    <div className="flex space-x-2 items-center">
      {children}
      <Label htmlFor={name} className="font-normal cursor-pointer">
        {label}
      </Label>
    </div>
    {!error && hint && <FormHint>{hint}</FormHint>}
    {error && <FormError error={error} />}
  </div>
)

export const FormHint = ({ children }: { children: React.ReactNode }) => <Info>{children}</Info>

export const FormError = ({ error }: { error: string | undefined }) => <Error>{error}</Error>
