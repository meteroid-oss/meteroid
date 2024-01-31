import { ButtonAlt, Dropdown, PopoverAlt } from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { StyledFamilyPicker } from '@/components/atoms/FamilyPicker/FamilyPicker.styled'
import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

import type { FunctionComponent } from 'react'

const FamilyPicker: FunctionComponent = () => {
  const familiesQuery = useQuery(listProductFamilies)
  const families = familiesQuery.data?.productFamilies ?? []

  // TODO query params ?
  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()
  const selected = families.find(fam => fam.externalId === familyExternalId) || families[0]

  return (
    <StyledFamilyPicker>
      {selected ? (
        <Dropdown
          side="bottom"
          align="start"
          overlay={
            <>
              {families
                .sort((a, b) => a.name.localeCompare(b.name))
                .map(fam => (
                  <Link key={fam.id} to={fam.externalId}>
                    <Dropdown.Item>{fam.name}</Dropdown.Item>
                  </Link>
                ))}
              <PopoverAlt.Separator />
              <Dropdown.Item icon={<PlusIcon size="12" />} onClick={() => alert('TODO')}>
                Manage
              </Dropdown.Item>
            </>
          }
        >
          <ButtonAlt
            as="span"
            type="text"
            size="tiny"
            className="my-1"
            loading={familiesQuery.isLoading}
          >
            {selected?.name}
          </ButtonAlt>
        </Dropdown>
      ) : (
        <ButtonAlt
          type="alternative"
          size="small"
          className="my-1 ml-1"
          loading={familiesQuery.isLoading}
        >
          Create a family
        </ButtonAlt>
      )}
    </StyledFamilyPicker>
  )
}

export default FamilyPicker
