export const getCountryFlagEmoji = (countryCode: string) => {
  const codePoints = countryCode.split('').map(char => 127397 + char.charCodeAt(0))
  return String.fromCodePoint(...codePoints)
}
