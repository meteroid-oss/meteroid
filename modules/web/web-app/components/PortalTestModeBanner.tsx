import type { CSSProperties } from 'react'

const MESSAGE = 'Test mode – Not a real document'

const PILL: CSSProperties = {
  position: 'fixed',
  top: 0,
  left: '50%',
  transform: 'translateX(-50%)',
  zIndex: 1000,
  pointerEvents: 'none',
  borderRadius: '0 0 6px 6px',
  borderTop: 'none',
  padding: '2px 10px',
}

const STRIP: CSSProperties = {
  alignSelf: 'flex-end',
  width: 'fit-content',
  marginLeft: 'auto',
  borderRadius: 4,
  padding: '1px 8px',
  marginBottom: 8,
}

const COLORS: CSSProperties = {
  background: '#fef3c7',
  color: '#92400e',
  border: '1px solid #fcd34d',
  fontSize: 11,
  fontWeight: 500,
  lineHeight: 1.4,
}

/** Customer-facing pages of a non-production workspace say so. `inline` for embedded widgets. */
export const PortalTestModeBanner = ({
  active,
  inline = false,
}: {
  active?: boolean
  inline?: boolean
}) =>
  active ? (
    <div role="status" style={{ ...COLORS, ...(inline ? STRIP : PILL) }}>
      {MESSAGE}
    </div>
  ) : null
