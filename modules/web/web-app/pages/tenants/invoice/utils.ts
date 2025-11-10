
// Function to resize SVG content by manipulating viewBox and dimensions
export const resizeSvgContent = (html: string, scaleFactor: number = 0.8): string => {
  // Create a temporary DOM parser to work with the HTML
  const parser = new DOMParser()
  const doc = parser.parseFromString(html, 'text/html')
  const svgElement = doc.querySelector('svg')

  if (!svgElement) {
    console.warn('No SVG element found in the provided HTML.')
    return html
  }

  // Get current dimensions
  const width = svgElement.getAttribute('width')
  const height = svgElement.getAttribute('height')

  // Scale dimensions if they exist, removing units like 'pt', 'px', etc.
  if (width && !width.includes('%')) {
    const numWidth = parseFloat(width)
    if (!isNaN(numWidth)) {
      // Remove units and set as unitless number (defaults to pixels)
      svgElement.setAttribute('width', (numWidth * scaleFactor).toString())
    }
  }

  if (height && !height.includes('%')) {
    const numHeight = parseFloat(height)
    if (!isNaN(numHeight)) {
      // Remove units and set as unitless number (defaults to pixels)
      svgElement.setAttribute('height', (numHeight * scaleFactor).toString())
    }
  }
  return doc.documentElement.outerHTML
}
