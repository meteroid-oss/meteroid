interface FramedLocation {
  origin: string
  ancestorOrigins?: ArrayLike<string>
}

/**
 * The origins of the documents framing the portal, innermost first, as the browser reports them.
 * Without `ancestorOrigins` (Firefox) the framing page is the referrer; empty when unknown.
 */
export const ancestorOrigins = (
  location: FramedLocation = window.location,
  referrer: string = document.referrer
): string[] => {
  if (location.ancestorOrigins && location.ancestorOrigins.length > 0) {
    return Array.from(location.ancestorOrigins)
  }
  try {
    const origin = new URL(referrer).origin
    return origin === location.origin ? [] : [origin]
  } catch {
    return []
  }
}

/** Embeddable anywhere with the plan, else only inside the origins the merchant registered. */
export const embeddingPermitted = (
  allowedAnywhere: boolean,
  registered: readonly string[],
  ancestors: readonly string[]
): boolean =>
  allowedAnywhere || (ancestors.length > 0 && ancestors.every(o => registered.includes(o)))
