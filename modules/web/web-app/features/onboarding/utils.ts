export const getCountryFlagEmoji = (countryCode: string) => {
  const codePoints = countryCode.split('').map(char => 127397 + char.charCodeAt(0))
  return String.fromCodePoint(...codePoints)
}

export const getCountryName = (countryCode: string, locale: string = 'en') => {
  try {
    const displayNames = new Intl.DisplayNames([locale], { type: 'region' })
    return displayNames.of(countryCode) ?? countryCode
  } catch {
    return countryCode
  }
}
