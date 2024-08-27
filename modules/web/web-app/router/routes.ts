export const routes = {
  organization(slug: string) {
    return `/${slug}`
  },
  tenantDetail(slug: string) {
    return `/tenant/${slug}`
  },
}
