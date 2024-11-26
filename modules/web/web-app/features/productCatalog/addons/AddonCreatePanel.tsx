import { FunctionComponent } from 'react'

import { createAddOn, listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { InputFormField } from '@ui/components'
import { useNavigate } from 'react-router-dom'

export const AddonCreatePanel: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const createAddonMut = useMutation(createAddOn, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listAddOns) })
    },
  })

  const methods = useZodForm({
    schema: schemas.addons.createAddonSchema,
  })

  return (
    <CatalogEditPanel
      visible={true}
      closePanel={() => navigate('..')}
      title={'Create addon'}
      methods={methods}
      onSubmit={a =>
        createAddonMut
          .mutateAsync({
            name: a.name,

            // productFamilyLocalId: familyLocalId,
            // fee TODO
          })
          .then(() => void 0)
      }
    >
      <div>
        <section className="space-y-4">
          <div className="space-y-6 py-2">
            <InputFormField
              name="name"
              label="Name"
              layout="horizontal"
              control={methods.control}
              type="text"
              placeholder="Plan name"
            />
            {/* 
            <SelectFormField
              name="productFamilyLocalId"
              label="Product line"
              layout="horizontal"
              placeholder="Select a product line"
              className="max-w-[320px]  "
              empty={families.length === 0}
              control={methods.control}
            >
              {families.map(f => (
                <SelectItem value={f.localId} key={f.localId}>
                  {f.name}
                </SelectItem>
              ))}
            </SelectFormField> */}

            {/* TODO */}
          </div>
        </section>
      </div>
    </CatalogEditPanel>
  )
}
