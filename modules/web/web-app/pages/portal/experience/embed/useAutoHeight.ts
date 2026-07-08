import { useEffect, useRef } from 'react'

/**
 * Wires a ResizeObserver to the returned ref and posts the document height to
 * the parent window whenever the embedded content reflows.
 *
 * The host SDK (`@md/portal-embed`) listens for `meteroid:resize` messages and
 * grows the iframe so the embed never scrolls or clips. We measure
 * `documentElement.scrollHeight` (rather than the ref's own height) so margins,
 * overlays and fonts loading late are all accounted for.
 */
export const useAutoHeight = <T extends HTMLElement = HTMLDivElement>() => {
  const ref = useRef<T>(null)

  useEffect(() => {
    if (typeof window === 'undefined' || window.parent === window) return

    let last = -1
    const post = () => {
      const height = Math.ceil(document.documentElement.scrollHeight)
      if (height === last) return
      last = height
      window.parent.postMessage({ type: 'meteroid:resize', height }, '*')
    }

    // Initial measure (after layout) + observe future reflows.
    post()
    const observer = new ResizeObserver(post)
    if (ref.current) observer.observe(ref.current)
    observer.observe(document.documentElement)
    window.addEventListener('resize', post)

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', post)
    }
  }, [])

  return ref
}
