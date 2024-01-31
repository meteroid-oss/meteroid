import { forwardRef, useImperativeHandle, useRef } from 'react'

import { cn } from '@ui/lib/cn'

import { twButtonAltStyles } from './Button.styles'

export interface ButtonProps extends React.HTMLAttributes<HTMLButtonElement> {
  block?: boolean
  className?: string
  children?: React.ReactNode
  disabled?: boolean
  onClick?: React.MouseEventHandler<HTMLButtonElement>
  icon?: React.ReactNode
  iconRight?: React.ReactNode
  loading?: boolean
  shadow?: boolean
  size?: 'tiny' | 'small' | 'medium' | 'large' | 'xlarge'
  style?: React.CSSProperties
  type?:
    | 'primary'
    | 'default'
    | 'secondary'
    | 'alternative'
    | 'outline'
    | 'dashed'
    | 'link'
    | 'text'
    | 'danger'
    | 'warning'
  htmlType?: 'button' | 'submit' | 'reset'
  ariaSelected?: boolean
  ariaControls?: string
  tabIndex?: 0 | -1
  role?: string
  as?: keyof JSX.IntrinsicElements
  form?: string
}

interface RefHandle {
  // container: () => HTMLElement | null
  button: () => HTMLButtonElement | null
}

export const ButtonAlt = forwardRef<RefHandle, ButtonProps>(
  (
    {
      block,
      className,
      children,
      disabled = false,
      onClick,
      icon,
      iconRight,
      loading = false,
      shadow = true,
      size = 'tiny',
      style,
      type = 'primary',
      htmlType = 'button',
      ariaSelected,
      ariaControls,
      tabIndex,
      role,
      as,
      ...props
    }: ButtonProps,
    ref
  ) => {
    // button ref
    // const containerRef = useRef<HTMLElement>(null)
    const buttonRef = useRef<HTMLButtonElement>(null)

    useImperativeHandle(ref, () => ({
      button: () => {
        return buttonRef.current
      },
    }))

    const __styles = twButtonAltStyles.button

    // styles
    const showIcon = loading || icon

    const classes = [__styles.base]
    const containerClasses = [__styles.container]

    classes.push(__styles.type[type])

    if (block) {
      containerClasses.push(__styles.block)
      classes.push(__styles.block)
    }

    if (shadow && type !== 'link' && type !== 'text') {
      classes.push(__styles.shadow)
    }

    if (size) {
      classes.push(__styles.size[size])
    }

    if (className) {
      classes.push(className)
    }

    if (disabled) {
      classes.push(__styles.disabled)
    }

    // custom button tag
    const CustomButton = ({ ...props }) => {
      const Tag = as as keyof JSX.IntrinsicElements
      return <Tag {...props} />
    }

    const buttonContent = (
      <>
        {showIcon && icon}
        {children && <span className={cn(__styles.label)}>{children}</span>}
        {iconRight && !loading && iconRight}
      </>
    )

    if (as) {
      return (
        <CustomButton {...props} className={cn(classes)} onClick={onClick} style={style}>
          {buttonContent}
        </CustomButton>
      )
    } else {
      return (
        // <span ref={containerRef} className={containerClasses.join(' ')}>
        <button
          {...props}
          ref={buttonRef}
          className={cn(classes)}
          disabled={loading || (disabled && true)}
          onClick={onClick}
          style={style}
          type={htmlType}
          aria-selected={ariaSelected}
          aria-controls={ariaControls}
          tabIndex={tabIndex}
          role={role}
          form={props.form}
        >
          {buttonContent}
        </button>
        // </span>
      )
    }
  }
)
