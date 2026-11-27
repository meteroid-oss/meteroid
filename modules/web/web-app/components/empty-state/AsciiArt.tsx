import { useEffect, useRef, type CSSProperties } from 'react'

import { scenes, toLayers, type SceneName } from './renderers'

/** A pose that reads well as a still, for prefers-reduced-motion. */
const STILL_T: Record<SceneName, number> = {
  customers: 0,
  plans: 0,
  invoices: 170,
  events: 8,
  reports: 40,
  subscriptions: 40,
  products: 35,
  metrics: 30,
  features: 30,
  addons: 90,
  wallet: 130,
}

export interface AsciiArtProps {
  scene: SceneName
  /**
   * Colour of the brightest characters, e.g. amber #f0b462 or green #7bd0a2. By default the
   * theme's foreground, so the art reads in light mode too.
   */
  tint?: string
  scanlines?: boolean
  /** 1 = default speed. */
  speed?: number
  /** Character size in px; the art is 44 x 22 characters. */
  fontSize?: number
  className?: string
}

const TICK_MS = 70 // ~14 fps: the low frame rate is part of the retro feel

export function AsciiArt({
  scene,
  tint = 'hsl(var(--foreground))',
  // Off by default: on the flat page background the lines read as a pattern laid over it.
  scanlines = false,
  speed = 1,
  fontSize = 11,
  className,
}: AsciiArtProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const dimRef = useRef<HTMLPreElement>(null)
  const midRef = useRef<HTMLPreElement>(null)
  const brightRef = useRef<HTMLPreElement>(null)

  useEffect(() => {
    const render = scenes[scene]
    const draw = (t: number) => {
      const [dim, mid, bright] = toLayers(render(t))
      // Write straight to the DOM: no React re-render 14 times a second.
      if (dimRef.current) dimRef.current.textContent = dim
      if (midRef.current) midRef.current.textContent = mid
      if (brightRef.current) brightRef.current.textContent = bright
    }

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduced) {
      draw(STILL_T[scene])
      return
    }

    let t = 0
    let visible = true
    let timer: number | undefined
    const tick = () => {
      t += speed
      draw(t)
    }
    const start = () => {
      if (timer === undefined && visible && !document.hidden)
        timer = window.setInterval(tick, TICK_MS)
    }
    const stop = () => {
      if (timer !== undefined) window.clearInterval(timer)
      timer = undefined
    }

    draw(t)
    start()

    // Pause when scrolled out of view or when the tab is in the background.
    const io = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting
      if (visible) start()
      else stop()
    })
    if (rootRef.current) io.observe(rootRef.current)
    const onVisibility = () => (document.hidden ? stop() : start())
    document.addEventListener('visibilitychange', onVisibility)

    return () => {
      stop()
      io.disconnect()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [scene, speed])

  const lineHeight = Math.round(fontSize * 1.09)
  const layer: CSSProperties = {
    position: 'absolute',
    left: '50%',
    top: '50%',
    transform: 'translate(-50%, -50%)',
    margin: 0,
    fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
    fontSize,
    lineHeight: `${lineHeight}px`,
    whiteSpace: 'pre',
    userSelect: 'none',
  }

  return (
    <div
      ref={rootRef}
      aria-hidden="true"
      className={className}
      style={{
        position: 'relative',
        width: Math.round(fontSize * 0.6 * 44) + 30,
        height: lineHeight * 22 + 8,
      }}
    >
      {/* Dim and mid are the foreground at 20% and 50%: #3a3a3a and #8a8a8a on the dark page. */}
      <pre ref={dimRef} style={{ ...layer, color: 'hsl(var(--foreground) / 0.2)' }} />
      <pre ref={midRef} style={{ ...layer, color: 'hsl(var(--foreground) / 0.5)' }} />
      <pre
        ref={brightRef}
        style={{
          ...layer,
          color: tint,
          // The glow scales with the characters: 8px at the default 11px.
          textShadow: `0 0 ${Math.round(fontSize * 0.73)}px color-mix(in srgb, ${tint} 50%, transparent)`,
        }}
      />
      {scanlines && (
        // The lines are the page's own background, so they disappear into it in any theme.
        <div
          style={{
            position: 'absolute',
            inset: 0,
            pointerEvents: 'none',
            background:
              'repeating-linear-gradient(to bottom, hsl(var(--background) / 0.55) 0px, hsl(var(--background) / 0.55) 1px, transparent 1px, transparent 3px)',
          }}
        />
      )}
    </div>
  )
}
