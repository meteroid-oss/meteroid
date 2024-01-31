import { XCircle } from 'lucide-react'
import { useState } from 'react'
import { FieldValues, Control, Controller, ControllerRenderProps, Path } from 'react-hook-form'

export type EditableTableCellProps<T extends FieldValues> = {
  control: Control<T>
  name: Path<T>
  isPrice: boolean
  precision?: number
  currency: string
  disabled?: boolean
  placeholder?: string
  onDelete?: () => void
}

export const EditableTableCell = <T extends FieldValues>({
  control,
  name,
  isPrice,
  currency,
  disabled = false,
  placeholder = '',
  onDelete,
}: EditableTableCellProps<T>) => {
  const [isEditing, setIsEditing] = useState(false)

  const renderTextInput = ({ field }: { field: ControllerRenderProps<T, Path<T>> }) => (
    <input
      {...field}
      type="text"
      disabled={disabled}
      className="w-full px-3 text-sm"
      placeholder={placeholder}
      onFocus={() => setIsEditing(true)}
      onBlur={() => setIsEditing(false)}
    />
  )

  const renderPriceInput = ({ field }: { field: ControllerRenderProps<T, Path<T>> }) => (
    <div className="relative">
      {currency && (
        <div className="absolute left-0 pl-3 flex items-center">
          <span className="text-sm">{currency}</span>
        </div>
      )}
      <input
        {...field}
        type="text"
        disabled={disabled}
        className="w-full px-3 text-sm pl-8"
        placeholder={placeholder}
        onFocus={() => setIsEditing(true)}
        onBlur={() => setIsEditing(false)}
      />
    </div>
  )

  return (
    <td className={`relative text-sm ${isEditing ? 'bg-gray-100' : ''}`}>
      <Controller
        control={control}
        name={name}
        render={({ field }) => (isPrice ? renderPriceInput({ field }) : renderTextInput({ field }))}
      />
      {onDelete && (
        <XCircle onClick={onDelete} className="absolute right-2 top-2 cursor-pointer h-4 w-4" />
      )}
    </td>
  )
}
