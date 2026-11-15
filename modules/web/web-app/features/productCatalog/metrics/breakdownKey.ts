/**
 * The breakdown key a duplicated metric starts with. Without the field on the form the user could
 * neither see nor clear a copied key, and creating the metric with it would be refused.
 */
export const copiedBreakdownKey = (
  sourceKey: string | undefined,
  fieldShown: boolean
): string | undefined => (fieldShown ? sourceKey || undefined : undefined)
