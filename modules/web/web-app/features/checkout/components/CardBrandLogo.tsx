interface CardBrandLogoProps {
  brand?: string
}

export const CardBrandLogo: React.FC<CardBrandLogoProps> = ({ brand }) => {
  if (!brand) return null

  const brandToLogoClass: Record<string, string> = {
    visa: 'bg-blue-100 text-blue-800',
    mastercard: 'bg-orange-100 text-orange-800',
    amex: 'bg-green-100 text-green-800',
    discover: 'bg-purple-100 text-purple-800',
    default: 'bg-gray-100 text-gray-800',
  }

  const logoClass = brandToLogoClass[brand.toLowerCase()] || brandToLogoClass.default

  return (
    <div className={`rounded px-2 py-1 text-xs font-medium ${logoClass}`}>
      {brand.charAt(0).toUpperCase() + brand.slice(1)}
    </div>
  )
}
