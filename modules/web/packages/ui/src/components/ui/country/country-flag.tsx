import { Flex } from '@ui/components/ui/flex'

const getCountryName = (code: string): string => {
  try {
    const displayNames = new Intl.DisplayNames(['en'], { type: 'region' })
    return displayNames.of(code) ?? code
  } catch {
    return code
  }
}

export const CountryFlag = ({ name }: { name?: string }) => {
  if (!name) {
    return null
  }

  // Assume it's a country code (2 letters)
  const countryCode = name.toUpperCase()
  const countryName = getCountryName(countryCode)

  return (
    <Flex align="center" className="gap-2">
      <img
        src={`https://flagcdn.com/w40/${countryCode.toLowerCase()}.png`}
        width="14"
        alt={countryName}
      />
      <div>{countryName}</div>
    </Flex>
  )
}
