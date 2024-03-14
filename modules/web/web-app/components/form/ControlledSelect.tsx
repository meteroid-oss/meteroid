import { SelectContent, SelectTrigger, SelectValue } from '@md/ui'
import * as SelectPrimitive from '@radix-ui/react-select'
import {
  useController,
  type Control,
  type FieldValues,
  type UseControllerProps,
} from 'react-hook-form'

type SelectProps<T extends FieldValues> = Omit<
  SelectPrimitive.SelectProps,
  'name' | 'onValueChange' | 'ref' | 'value'
> &
  UseControllerProps<T> & {
    placeholder?: string
    children: React.ReactNode
    className?: string
    control: Control<T>
  }

export const ControlledSelect = <T extends FieldValues>({
  children,
  ...props
}: SelectProps<T>): JSX.Element => {
  const { field } = useController(props)

  return (
    <SelectPrimitive.Root
      {...props}
      name={field.name}
      onValueChange={field.onChange}
      value={field.value}
    >
      <SelectTrigger ref={field.ref} className={props.className}>
        <SelectValue placeholder={props.placeholder} />
      </SelectTrigger>
      <SelectContent className={props.className}>{children}</SelectContent>
    </SelectPrimitive.Root>
  )
}
