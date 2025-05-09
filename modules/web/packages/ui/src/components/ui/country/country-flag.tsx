import { countries } from '@ui/components/ui/country/data'
import { Flex } from '@ui/components/ui/flex'

export const CountryFlag = ({ name }: { name?: string }) => {
  const country = name ? Object.values(countries).find(c => c.name === name) : undefined

  return country ? (
    <Flex align="center" className="gap-2">
      <img
        src={`https://flagcdn.com/w40/${country.code.toLowerCase()}.png`}
        width="14"
        alt={country.name}
      />
      <div>{country.name}</div>
    </Flex>
  ) : (
    <div>{name}</div>
  )
}
