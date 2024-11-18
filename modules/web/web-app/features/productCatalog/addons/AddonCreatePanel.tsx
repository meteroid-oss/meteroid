import { FunctionComponent } from 'react'

// import { createAddOn, listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

// import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
// import { useZodForm } from '@/hooks/useZodForm'
// import { schemas } from '@/lib/schemas'
// import { useTypedParams } from '@/utils/params'
// import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
// import { useQueryClient } from '@tanstack/react-query'
// import { useNavigate } from 'react-router-dom'

export const AddonCreatePanel: FunctionComponent = () => {
  return <>Not implemented</>
  // const queryClient = useQueryClient()
  // const navigate = useNavigate()
  // const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

  // const createAddonMut = useMutation(createAddOn, {
  //   onSuccess: async () => {
  //     await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listAddOns) })
  //   },
  // })

  // const createMethods = useZodForm({
  //   schema: schemas.addons.createAddonSchema,
  // })

  // return (
  //   <CatalogEditPanel
  //     visible={true}
  //     closePanel={() => navigate('..')}
  //     title={'Create addon'}
  //     methods={createMethods}
  //     onSubmit={a =>
  //       createAddonMut
  //         .mutateAsync({
  //           name: a.name,
  //           // productFamilyLocalId: familyLocalId,
  //           // fee TODO
  //         })
  //         .then(() => void 0)
  //     }
  //   >
  //     test
  //   </CatalogEditPanel>
  // )
}
