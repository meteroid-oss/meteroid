export const getPageRange = (currentPage: number, totalPages: number) => {
  const visiblePages = 3 // Number of visible pages on each side of the current page
  const rangeStart = Math.max(1, currentPage - visiblePages)
  const rangeEnd = Math.min(totalPages, currentPage + visiblePages)

  const pages = []
  let ellipsisStart = false
  let ellipsisEnd = false

  for (let i = 1; i <= totalPages; i++) {
    if (i === 1 || i === totalPages || (i >= rangeStart && i <= rangeEnd)) {
      pages.push(i)
      ellipsisStart = false
      ellipsisEnd = false
    } else {
      if (!ellipsisStart && i < rangeStart) {
        pages.push('...')
        ellipsisStart = true
      } else if (!ellipsisEnd && i > rangeEnd) {
        pages.push('...')
        ellipsisEnd = true
      }
    }
  }

  return pages
}
