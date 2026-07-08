import { CSSProperties, ReactNode, useEffect } from 'react'

import { StatusBadge } from './format'

import './portal.css'

/* ------------------------------------------------------------------ surfaces */

interface CardProps {
  children: ReactNode
  style?: CSSProperties
  className?: string
  /** Padding shorthand; defaults to 22px. */
  pad?: number | string
  onClick?: () => void
}

export const Card = ({ children, style, className, pad = 22, onClick }: CardProps) => (
  <div
    className={className}
    onClick={onClick}
    style={{
      background: 'var(--mtp-surface)',
      border: '1px solid var(--mtp-border)',
      borderRadius: 'var(--mtp-r-card)',
      padding: pad,
      ...style,
    }}
  >
    {children}
  </div>
)

/** The accent-tinted "spotlight" surface used for the headline plan card. */
export const SpotlightCard = ({ children, style, pad = 24 }: CardProps) => (
  <div
    style={{
      background: 'var(--mtp-spot)',
      color: 'var(--mtp-spot-text)',
      border: '1px solid var(--mtp-spot-border)',
      borderRadius: 'var(--mtp-r-card)',
      padding: pad,
      ...style,
    }}
  >
    {children}
  </div>
)

/** A card with a header strip and a flush body (used for lists/tables). */
export const PanelCard = ({
  title,
  action,
  children,
  style,
}: {
  title: ReactNode
  action?: ReactNode
  children: ReactNode
  style?: CSSProperties
}) => (
  <div
    style={{
      background: 'var(--mtp-surface)',
      border: '1px solid var(--mtp-border)',
      borderRadius: 'var(--mtp-r-card)',
      overflow: 'hidden',
      ...style,
    }}
  >
    <div
      style={{
        padding: '16px 20px',
        borderBottom: '1px solid var(--mtp-border)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
      }}
    >
      <span style={{ fontSize: 13.5, fontWeight: 600 }}>{title}</span>
      {action}
    </div>
    {children}
  </div>
)

export const Eyebrow = ({ children, spot }: { children: ReactNode; spot?: boolean }) => (
  <span
    style={{
      fontSize: 11.5,
      fontWeight: 600,
      letterSpacing: '0.06em',
      textTransform: 'uppercase',
      color: spot ? 'var(--mtp-spot-3)' : 'var(--mtp-text-3)',
    }}
  >
    {children}
  </span>
)

/* -------------------------------------------------------------------- badges */

const TONE: Record<StatusBadge['tone'], CSSProperties> = {
  ok: { color: 'var(--mtp-ok-text)', background: 'var(--mtp-ok-bg)' },
  neutral: { color: 'var(--mtp-text-2)', background: 'var(--mtp-track)' },
  warn: { color: 'var(--mtp-warn-text)', background: 'var(--mtp-warn-bg)' },
  danger: { color: 'var(--mtp-danger)', background: 'var(--mtp-danger-bg)' },
}

export const Pill = ({ badge, dot }: { badge: StatusBadge; dot?: boolean }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      fontSize: 11,
      fontWeight: 600,
      padding: '2px 9px',
      borderRadius: 20,
      whiteSpace: 'nowrap',
      ...TONE[badge.tone],
    }}
  >
    {dot && (
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: badge.tone === 'ok' ? 'var(--mtp-ok-dot)' : 'currentColor',
        }}
      />
    )}
    {badge.label}
  </span>
)

/* --------------------------------------------------------------------- meter */

export const Meter = ({
  pct,
  spot,
  height = 8,
}: {
  pct: number
  spot?: boolean
  height?: number
}) => {
  const danger = pct >= 90
  return (
    <div
      style={{
        height,
        borderRadius: 20,
        background: spot ? 'var(--mtp-spot-track)' : 'var(--mtp-track)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          width: `${Math.max(2, Math.min(100, pct))}%`,
          height: '100%',
          borderRadius: 'inherit',
          background: spot
            ? 'var(--mtp-spot-fill)'
            : danger
              ? 'var(--mtp-danger)'
              : 'var(--mtp-fill)',
        }}
      />
    </div>
  )
}

/* ------------------------------------------------------------------- buttons */

type BtnVariant = 'primary' | 'secondary' | 'ghost' | 'danger'

const BTN_BASE: CSSProperties = {
  fontFamily: 'inherit',
  cursor: 'pointer',
  fontWeight: 600,
  borderRadius: 'var(--mtp-r-ctrl)',
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  gap: 7,
  whiteSpace: 'nowrap',
}

const BTN_VARIANT: Record<BtnVariant, CSSProperties> = {
  primary: { background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)', border: 'none' },
  secondary: {
    background: 'transparent',
    color: 'var(--mtp-text)',
    border: '1px solid var(--mtp-border-2)',
    fontWeight: 500,
  },
  ghost: {
    background: 'none',
    color: 'var(--mtp-text-2)',
    border: 'none',
    fontWeight: 500,
  },
  danger: {
    background: 'transparent',
    color: 'var(--mtp-text-2)',
    border: '1px solid var(--mtp-border-2)',
    fontWeight: 500,
  },
}

export const PButton = ({
  children,
  variant = 'primary',
  onClick,
  disabled,
  loading,
  size = 'md',
  type = 'button',
  style,
}: {
  children: ReactNode
  variant?: BtnVariant
  onClick?: () => void
  disabled?: boolean
  loading?: boolean
  size?: 'sm' | 'md'
  type?: 'button' | 'submit'
  style?: CSSProperties
}) => {
  const pad = size === 'sm' ? '7px 12px' : '9px 16px'
  const fontSize = size === 'sm' ? 12.5 : 13
  const isDisabled = disabled || loading
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={isDisabled}
      className={`mtp-btn mtp-btn-${variant}`}
      style={{
        ...BTN_BASE,
        ...BTN_VARIANT[variant],
        padding: pad,
        fontSize,
        opacity: isDisabled && !loading ? 0.55 : 1,
        ...style,
      }}
    >
      {loading && <Spinner size={13} />}
      {children}
    </button>
  )
}

export const LinkButton = ({
  children,
  onClick,
  style,
}: {
  children: ReactNode
  onClick?: () => void
  style?: CSSProperties
}) => (
  <button
    type="button"
    onClick={onClick}
    className="mtp-link"
    style={{
      fontSize: 12.5,
      fontWeight: 500,
      color: 'var(--mtp-accent-ink)',
      background: 'none',
      border: 'none',
      cursor: 'pointer',
      fontFamily: 'inherit',
      padding: 0,
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      ...style,
    }}
  >
    {children}
  </button>
)

/* ------------------------------------------------------------------ typography */

export const Mono = ({
  children,
  style,
}: {
  children: ReactNode
  style?: CSSProperties
}) => (
  <span style={{ fontFamily: 'var(--mtp-mono)', ...style }}>{children}</span>
)

/* ---------------------------------------------------------------------- modal */

export const Modal = ({
  open,
  onClose,
  children,
  maxWidth = 460,
}: {
  open: boolean
  onClose: () => void
  children: ReactNode
  maxWidth?: number
}) => {
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onClose])

  if (!open) return null
  return (
    <div
      onClick={onClose}
      className="mtp-overlay"
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 50,
        background: 'var(--mtp-overlay)',
        backdropFilter: 'blur(4px)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        className="mtp-sheet mtp-scroll"
        style={{
          width: '100%',
          maxWidth,
          maxHeight: '90vh',
          overflowY: 'auto',
          background: 'var(--mtp-sheet)',
          color: 'var(--mtp-text)',
          border: '1px solid var(--mtp-border-2)',
          borderRadius: 'var(--mtp-r-card)',
          boxShadow: '0 30px 90px rgba(0,0,0,0.25)',
        }}
      >
        {children}
      </div>
    </div>
  )
}

export const ModalCloseButton = ({ onClose }: { onClose: () => void }) => (
  <button
    type="button"
    onClick={onClose}
    className="mtp-btn mtp-btn-secondary"
    style={{
      width: 30,
      height: 30,
      padding: 0,
      borderRadius: 'var(--mtp-r-sm)',
      color: 'var(--mtp-text-2)',
    }}
    aria-label="Close"
  >
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M18 6L6 18M6 6l12 12" />
    </svg>
  </button>
)

/* -------------------------------------------------------------------- stepper */

const StepBtn = ({
  children,
  onClick,
  disabled,
  ariaLabel,
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
  ariaLabel: string
}) => (
  <button
    type="button"
    aria-label={ariaLabel}
    onClick={onClick}
    disabled={disabled}
    className="mtp-btn"
    style={{
      width: 28,
      height: 28,
      borderRadius: 'var(--mtp-r-sm)',
      border: '1px solid var(--mtp-border-2)',
      background: 'var(--mtp-surface)',
      color: 'var(--mtp-text)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      cursor: 'pointer',
      fontFamily: 'inherit',
      opacity: disabled ? 0.4 : 1,
    }}
  >
    {children}
  </button>
)

/** Compact −/value/+ quantity stepper. Pessimistic: caller disables via `busy`. */
export const Stepper = ({
  value,
  min = 0,
  max,
  onChange,
  disabled,
  busy,
}: {
  value: number
  min?: number
  max?: number
  onChange: (next: number) => void
  disabled?: boolean
  busy?: boolean
}) => (
  <div style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
    <StepBtn
      ariaLabel="Decrease"
      disabled={disabled || busy || value <= min}
      onClick={() => onChange(value - 1)}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4">
        <path d="M5 12h14" />
      </svg>
    </StepBtn>
    <div
      style={{
        minWidth: 34,
        textAlign: 'center',
        fontFamily: 'var(--mtp-mono)',
        fontSize: 14,
        fontWeight: 600,
        color: 'var(--mtp-text)',
      }}
    >
      {busy ? <Spinner size={13} /> : value}
    </div>
    <StepBtn
      ariaLabel="Increase"
      disabled={disabled || busy || (max != null && value >= max)}
      onClick={() => onChange(value + 1)}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4">
        <path d="M12 5v14M5 12h14" />
      </svg>
    </StepBtn>
  </div>
)

/* -------------------------------------------------------------------- spinner */

export const Spinner = ({ size = 16 }: { size?: number }) => (
  <svg
    className="mtp-spin"
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    style={{ flex: `0 0 ${size}px` }}
  >
    <path d="M21 12a9 9 0 1 1-6.219-8.56" />
  </svg>
)

/* ------------------------------------------------------------------- feedback */

export const CenterState = ({
  icon,
  title,
  hint,
  action,
}: {
  icon?: ReactNode
  title: string
  hint?: string
  action?: ReactNode
}) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 10,
      padding: '48px 24px',
      textAlign: 'center',
    }}
  >
    {icon}
    <span style={{ fontSize: 15, fontWeight: 600 }}>{title}</span>
    {hint && (
      <span style={{ fontSize: 13, color: 'var(--mtp-text-2)', maxWidth: 360 }}>{hint}</span>
    )}
    {action}
  </div>
)

export const Divider = ({ style }: { style?: CSSProperties }) => (
  <div style={{ height: 1, background: 'var(--mtp-border)', ...style }} />
)
