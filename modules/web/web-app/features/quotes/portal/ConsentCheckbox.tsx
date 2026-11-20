import { Checkbox } from '@md/ui'

interface Props {
  text: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  disabled?: boolean
}

export const ConsentCheckbox = ({ text, checked, onCheckedChange, disabled }: Props) => (
  <label className="flex items-start gap-3 rounded-lg border border-border bg-muted/30 p-3 text-sm cursor-pointer">
    <Checkbox
      checked={checked}
      onCheckedChange={value => onCheckedChange(value === true)}
      disabled={disabled}
      className="mt-0.5"
      aria-required
    />
    <span>{text}</span>
  </label>
)
