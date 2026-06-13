import { zodResolver } from '@hookform/resolvers/zod'
import {
  Control,
  FieldPath,
  FieldValues,
  Resolver,
  useForm,
  UseFormProps,
  UseFormReturn,
} from 'react-hook-form'
import { z } from 'zod'

/* eslint-disable @typescript-eslint/no-explicit-any */
// In zod v4 `z.infer<z.ZodType>` resolves to `unknown` (it was `any` in v3),
// which is not assignable to react-hook-form's `FieldValues`. Intersecting with
// `FieldValues` keeps the concrete schema shape while guaranteeing assignability
// to `FieldValues` for both untyped (`z.ZodType`) and generic-wrapped schemas.
type Values<TSchema extends z.ZodType> = z.output<TSchema> & FieldValues

export function useZodForm<TSchema extends z.ZodType<any, any, any>>(
  props: Omit<UseFormProps<Values<TSchema>>, 'resolver'> & {
    schema: TSchema
  }
): Methods<TSchema> {
  const form = useForm<Values<TSchema>>({
    mode: 'onBlur',
    ...props,
    resolver: zodResolver(props.schema) as unknown as Resolver<Values<TSchema>>,
  })

  const withControl = (name: FieldPath<Values<TSchema>>) => ({
    control: form.control,
    name,
  })
  const withError = (name: FieldPath<Values<TSchema>>) => ({
    error: form.formState.errors[name]?.message as string | undefined,
  })

  return { ...form, withControl, withError }
}

export interface Methods<TSchema extends z.ZodType<any, any, any>>
  extends UseFormReturn<Values<TSchema>, any, Values<TSchema>> {
  withControl: (name: FieldPath<Values<TSchema>>) => {
    control: Control<Values<TSchema>, any>
    name: FieldPath<Values<TSchema>>
  }
  withError: (name: FieldPath<Values<TSchema>>) => { error: string | undefined }
}
