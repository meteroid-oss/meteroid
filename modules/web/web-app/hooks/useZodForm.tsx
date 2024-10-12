import { zodResolver } from '@hookform/resolvers/zod'
import { Control, FieldPath, useForm, UseFormProps, UseFormReturn } from 'react-hook-form'
import { z } from 'zod'

/* eslint-disable @typescript-eslint/no-explicit-any */
export function useZodForm<TSchema extends z.Schema<any, any>>(
  props: Omit<UseFormProps<z.infer<TSchema>>, 'resolver'> & {
    schema: TSchema
  }
): Methods<TSchema> {
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
    error: form.formState.errors[name]?.message as string | undefined,
  })

  return { ...form, withControl, withError }
}

/* eslint-disable @typescript-eslint/no-explicit-any */
export interface Methods<TSchema extends z.Schema<any, any>>
  extends UseFormReturn<z.TypeOf<TSchema>, any, z.TypeOf<TSchema>> {
  withControl: (name: FieldPath<z.TypeOf<TSchema>>) => {
    control: Control<z.TypeOf<TSchema>, any, z.TypeOf<TSchema>>
    name: FieldPath<z.TypeOf<TSchema>>
  }
  withError: (name: FieldPath<z.TypeOf<TSchema>>) => { error: string | undefined }
}
