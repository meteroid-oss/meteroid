import { Checkbox } from '@ui2/components'
import * as CheckboxPrimitive from '@radix-ui/react-checkbox'
import {
  useController,
  type Control,
  type FieldValues,
  type UseControllerProps,
} from 'react-hook-form'

type SelectProps<T extends FieldValues> = Omit<
  CheckboxPrimitive.CheckboxProps,
  'name' | 'checked' | 'ref' | 'onCheckedChange'
> &
  UseControllerProps<T> & {
    placeholder?: string
    className?: string
    control: Control<T>
  }

export const ControlledCheckbox = <T extends FieldValues>(props: SelectProps<T>): JSX.Element => {
  const { field } = useController(props)

  return (
    <Checkbox
      {...props}
      name={field.name}
      checked={field.value}
      onCheckedChange={field.onChange}
      ref={field.ref}
    />
  )
}
