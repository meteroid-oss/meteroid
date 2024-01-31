// import { forwardRef } from 'react'

// import {
//   Input as InputComponent,
//   InputContainer,
//   InputIcon,
// } from '@ui/components/Input/Input.styled'

// import type { InputHTMLAttributes } from 'react'

// export type InputProps = InputHTMLAttributes<HTMLInputElement> & {
//   width?: string | number
//   icon?: JSX.Element
//   iconPosition?: 'left' | 'right'
// }

// const Input = forwardRef<HTMLInputElement, InputProps>(
//   ({ width = '100%', icon, iconPosition = 'right', className, style, ...props }, ref) => {
//     return (
//       <InputContainer
//         style={{
//           width,
//         }}
//       >
//         <InputComponent
//           className={className}
//           ref={ref}
//           icon={iconPosition}
//           style={{
//             ...style,
//             width,
//           }}
//           {...props}
//         />
//         {icon && <InputIcon position={iconPosition}>{icon}</InputIcon>}
//       </InputContainer>
//     )
//   }
// )
// Input.displayName = 'Input'
import { AlertCircleIcon, CopyIcon } from 'lucide-react'
import { forwardRef, useState } from 'react'

import { twInputAltStyles } from '@ui/components/Input/Input.styles'
import { HIDDEN_PLACEHOLDER } from '@ui/lib/constants'

import { ButtonAlt as Button } from '../ButtonAlt'

export interface Props
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'size' | 'onCopy'> {
  copy?: boolean
  onCopy?: () => void
  defaultValue?: string | number
  disabled?: boolean
  error?: string
  icon?: React.ReactNode
  reveal?: boolean
  actions?: React.ReactNode
  size?: 'tiny' | 'small' | 'medium' | 'large' | 'xlarge'
}

const Input = forwardRef<HTMLInputElement, Props>(
  (
    {
      autoComplete,
      autoFocus,
      copy,
      defaultValue,
      disabled,
      error,
      icon,
      id = '',
      name = '',
      onChange,
      onBlur,
      onCopy,
      placeholder,
      type = 'text',
      value = undefined,
      reveal = false,
      actions,
      size = 'medium',
      className,
      ...props
    },
    ref
  ) => {
    const [copyLabel, setCopyLabel] = useState('Copy')
    const [hidden, setHidden] = useState(true)

    const __styles = twInputAltStyles.input

    function _onCopy(value: string | undefined) {
      value &&
        navigator.clipboard.writeText(value)?.then(
          function () {
            /* clipboard successfully set */
            setCopyLabel('Copied')
            setTimeout(function () {
              setCopyLabel('Copy')
            }, 3000)
            onCopy?.()
          },
          function () {
            /* clipboard write failed */
            setCopyLabel('Failed to copy')
          }
        )
    }

    function onReveal() {
      setHidden(false)
    }

    const inputClasses = [__styles.base, className]

    if (error) inputClasses.push(__styles.variants.error)
    if (!error) inputClasses.push(__styles.variants.standard)
    if (icon) inputClasses.push(__styles.with_icon)
    if (size) inputClasses.push(__styles.size[size])
    if (disabled) inputClasses.push(__styles.disabled)

    return (
      <div className={__styles.container}>
        <input
          autoComplete={autoComplete}
          autoFocus={autoFocus}
          defaultValue={defaultValue}
          disabled={disabled}
          id={id}
          name={name}
          onChange={onChange}
          onBlur={onBlur}
          onCopy={onCopy}
          placeholder={placeholder}
          ref={ref}
          type={type}
          value={reveal && hidden ? HIDDEN_PLACEHOLDER : value}
          className={inputClasses.join(' ')}
          {...props}
        />
        {icon && (
          <div className=" absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-scale-1100">
            {icon}
          </div>
        )}
        {copy || error || actions ? (
          <div className={__styles.actions_container}>
            {copy && !(reveal && hidden) ? (
              <Button
                size="tiny"
                type="default"
                icon={<CopyIcon />}
                onClick={() => _onCopy(value as string | undefined)}
              >
                {copyLabel}
              </Button>
            ) : null}
            {reveal && hidden ? (
              <Button size="tiny" type="default" onClick={onReveal}>
                Reveal
              </Button>
            ) : null}
            {actions && actions}
          </div>
        ) : null}
      </div>
    )
  }
)
export type InputProps = Props & React.RefAttributes<HTMLInputElement>

export { Input }
