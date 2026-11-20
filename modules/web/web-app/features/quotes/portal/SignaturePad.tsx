import { Button, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import SignatureCanvas from 'react-signature-canvas'

export type SignatureMode = 'typed' | 'drawn'

export const SIGNATURE_METHOD: Record<SignatureMode, string> = {
  typed: 'typed',
  drawn: 'digital_signature',
}

const SCRIPT_FONT = "'Dancing Script'"
const SYSTEM_SCRIPT_FONTS =
  "'Segoe Script', 'Bradley Hand', 'Snell Roundhand', 'Apple Chancery', 'Brush Script MT', 'URW Chancery L', 'Lucida Handwriting', cursive"

// The Latin and Latin Extended ranges of the self-hosted Dancing Script files (styles/main.css).
const SCRIPT_FONT_COVERAGE =
  /^[\u0020-\u02FF\u1D00-\u1DBF\u1E00-\u1EFF\u2000-\u206F\u20A0-\u20C0\u2113\u2122\u2C60-\u2C7F\uA720-\uA7FF]*$/

/** Whether Dancing Script has every character of the name; other scripts sign by hand by default. */
export const scriptFontCovers = (name: string) => SCRIPT_FONT_COVERAGE.test(name.normalize('NFC'))

// The image goes as-is into the signed PDF: dark ink on white paper whatever the portal theme.
const INK = '#111111'
const PAPER = '#ffffff'
const MAX_PIXEL_RATIO = 2

/** The name in a script font, as a PNG data URL, once the font is loaded. */
export const renderTypedSignature = async (name: string): Promise<string> => {
  const covered = scriptFontCovers(name)
  const family = covered ? `${SCRIPT_FONT}, ${SYSTEM_SCRIPT_FONTS}` : SYSTEM_SCRIPT_FONTS
  if (covered && document.fonts) {
    // A failed load falls through to the system script fonts of the stack.
    await document.fonts.load(`64px ${SCRIPT_FONT}`, name).catch(() => undefined)
  }
  const width = 600
  const height = 160
  const canvas = document.createElement('canvas')
  canvas.width = width * MAX_PIXEL_RATIO
  canvas.height = height * MAX_PIXEL_RATIO
  const ctx = canvas.getContext('2d')
  if (!ctx) return ''
  ctx.scale(MAX_PIXEL_RATIO, MAX_PIXEL_RATIO)
  ctx.fillStyle = PAPER
  ctx.fillRect(0, 0, width, height)

  let size = 64
  ctx.font = `${size}px ${family}`
  while (ctx.measureText(name).width > width - 48 && size > 16) {
    size -= 2
    ctx.font = `${size}px ${family}`
  }
  ctx.fillStyle = INK
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.fillText(name, width / 2, height / 2)
  return canvas.toDataURL('image/png')
}

const DrawPad = ({ onChange }: { onChange: (dataUrl: string) => void }) => {
  const padRef = useRef<SignatureCanvas>(null)
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  // Fits the bitmap to the displayed size (and pixel ratio), redrawing the strokes already made.
  const fit = useCallback(() => {
    const pad = padRef.current
    if (!pad) return
    const canvas = pad.getCanvas()
    const ratio = Math.min(Math.max(window.devicePixelRatio || 1, 1), MAX_PIXEL_RATIO)
    const width = Math.round(canvas.offsetWidth * ratio)
    const height = Math.round(canvas.offsetHeight * ratio)
    if (!width || !height || (canvas.width === width && canvas.height === height)) return
    const strokes = pad.toData()
    canvas.width = width
    canvas.height = height
    canvas.getContext('2d')?.scale(ratio, ratio)
    pad.clear()
    if (strokes.length > 0) {
      pad.fromData(strokes)
      onChangeRef.current(pad.toDataURL('image/png'))
    }
  }, [])

  useEffect(() => {
    const canvas = padRef.current?.getCanvas()
    if (!canvas) return
    fit()
    const observer = new ResizeObserver(fit)
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [fit])

  const clear = () => {
    padRef.current?.clear()
    onChange('')
  }

  return (
    <div className="space-y-2">
      <div className="overflow-hidden rounded-lg border border-border">
        <SignatureCanvas
          ref={padRef}
          penColor={INK}
          backgroundColor={PAPER}
          throttle={8}
          clearOnResize={false}
          canvasProps={{
            className: 'block h-40 w-full touch-none',
            role: 'img',
            'aria-label': 'Draw your signature',
          }}
          onEnd={() => padRef.current && onChange(padRef.current.toDataURL('image/png'))}
        />
      </div>
      <div className="flex justify-end">
        <Button type="button" variant="outline" size="sm" onClick={clear}>
          Clear
        </Button>
      </div>
    </div>
  )
}

interface Props {
  name: string
  mode: SignatureMode
  onModeChange: (mode: SignatureMode) => void
  onDrawnChange: (dataUrl: string) => void
  error?: string
}

export const SignaturePad = ({ name, mode, onModeChange, onDrawnChange, error }: Props) => {
  const [typed, setTyped] = useState('')

  useEffect(() => {
    const trimmed = name.trim()
    if (!trimmed) {
      setTyped('')
      return
    }
    let cancelled = false
    void renderTypedSignature(trimmed).then(url => !cancelled && setTyped(url))
    return () => {
      cancelled = true
    }
  }, [name])

  return (
    <Tabs value={mode} onValueChange={value => onModeChange(value as SignatureMode)}>
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium text-foreground">Signature *</span>
        <TabsList aria-label="Signature method">
          <TabsTrigger value="typed">Type</TabsTrigger>
          <TabsTrigger value="drawn">Draw</TabsTrigger>
        </TabsList>
      </div>
      <TabsContent value="typed" className="space-y-2">
        <div className="flex h-40 items-center justify-center overflow-hidden rounded-lg border border-border">
          {typed && name.trim() && (
            <img
              src={typed}
              alt={`Typed signature: ${name.trim()}`}
              className="h-full w-full object-contain"
            />
          )}
        </div>
      </TabsContent>
      {/* Kept mounted so a drawing survives switching tabs. */}
      <TabsContent value="drawn" forceMount className="data-[state=inactive]:hidden">
        <DrawPad onChange={onDrawnChange} />
      </TabsContent>
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
    </Tabs>
  )
}
