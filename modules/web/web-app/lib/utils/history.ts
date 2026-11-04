// The router numbers the entries of this document's history stack; a deep link is entry 0.
export const historyIdx = (): number | undefined => window.history.state?.idx
