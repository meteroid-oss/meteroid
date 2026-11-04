import { InputFormField } from '@md/ui'
import { KeyboardEvent as ReactKeyboardEvent } from 'react'
import { Control } from 'react-hook-form'

import { ConnectContext, ConnectDefinition } from './providers'

export type CredentialValues = Record<string, string | undefined>

/**
 * The provider's credential inputs from the registry. In `update` mode every field may stay
 * blank to keep its stored value, and the key hint stands in for the secret.
 */
export const CredentialFields = ({
  connect,
  fields,
  control,
  ctx,
  mode,
  keyHint,
  onSubmit,
}: {
  connect: ConnectDefinition
  fields: readonly string[]
  control: Control<CredentialValues>
  ctx: ConnectContext
  mode: 'create' | 'update'
  keyHint?: string
  /** Enter on the last field. */
  onSubmit?: () => void
}) => {
  const onKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>, idx: number) => {
    if (e.key !== 'Enter' || idx !== fields.length - 1 || !onSubmit) return
    e.preventDefault()
    onSubmit()
  }

  return (
    <div className="space-y-5">
      {fields.map((name, idx) => {
        const field = connect.fields[name]
        const placeholder =
          mode === 'update'
            ? 'Leave blank to keep the current value'
            : typeof field.placeholder === 'function'
              ? field.placeholder(ctx)
              : field.placeholder
        const description =
          mode === 'update' && field.keyHint && keyHint ? `Current: ${keyHint}` : field.help
        return (
          <InputFormField
            key={name}
            control={control}
            label={field.label}
            name={name}
            layout="vertical"
            description={description}
            placeholder={placeholder}
            showPasswordToggle={field.secret}
            data-form-type="other"
            autoComplete="off"
            onKeyDown={ev => onKeyDown(ev, idx)}
          />
        )
      })}
    </div>
  )
}
