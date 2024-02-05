import { zodResolver } from '@hookform/resolvers/zod'
import { FieldPath, useForm, UseFormProps } from 'react-hook-form'
import { z } from 'zod'

export function useZodForm<TSchema extends z.ZodTypeAny>(
  props: Omit<UseFormProps<z.infer<TSchema>>, 'resolver'> & {
    schema: TSchema
  }
) {
  const form = useForm({
    mode: 'onBlur',
    ...props,
    resolver: zodResolver(props.schema, undefined),
  })

  const withControl = (name: FieldPath<z.TypeOf<TSchema>>) => ({
    control: form.control,
    name,
  })
  const withError = (name: FieldPath<z.TypeOf<TSchema>>) => ({
    error: form.formState.errors[name]?.message,
  })

  return { ...form, withControl, withError }
}

export type Methods<TSchema extends z.ZodTypeAny> = ReturnType<typeof useZodForm<TSchema>>
