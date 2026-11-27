/** Archived scope, orthogonal to lifecycle. Maps to the list request `archived`
 * field: active -> false, archived -> true, all -> undefined. */
export type ArchivedScope = 'active' | 'archived' | 'all'
