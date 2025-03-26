import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { useWatch } from 'react-hook-form'

import { Methods } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

export const AccountingCurrencySelect = ({
  methods,
}: {
  methods: Methods<typeof schemas.organizations.organizationOnboardingSchema>
}) => {
  const getCountriesQuery = useQuery(getCountries)
  const country = useWatch({
    name: 'country',
    control: methods.control,
  })

  const countryData = getCountriesQuery.data?.countries.find(c => c.code === country)

  return (
    <Select value={countryData?.currency}>
      <SelectTrigger disabled={true}>
        <SelectValue placeholder="Select a country" />
      </SelectTrigger>
      <SelectContent hideWhenDetached>
        {countryData?.currency && (
          <SelectItem value={countryData.currency}>{countryData.currency}</SelectItem>
        )}
      </SelectContent>
    </Select>
  )
}
