import { atom, useAtom } from 'jotai'
import { useEffect } from 'react'

import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { listInvoicingEntities } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'


interface InvEntityWithTenant {
  id: string
  tenantId: string
}

const invoicingEntityAtom = atom<InvEntityWithTenant | undefined>(undefined)

export const useInvoicingEntity = () => {
  const { tenant } = useTenant() 

  const [selectedEntity, setSelectedEntity] = useAtom(invoicingEntityAtom)
  
  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)
  
  const entities = listInvoicingEntitiesQuery.data?.entities ?? []
  const defaultEntity = entities.find(entity => entity.isDefault)
  const currentEntity = entities.find(entity => entity.id === selectedEntity?.id)


  useEffect(() => {
    setSelectedEntity(a => (a?.tenantId === tenant?.id ? a : undefined))
  }, [tenant])
  
  // Set default entity if no selection exists and default is available
  useEffect(() => {
    if (!selectedEntity && defaultEntity && !listInvoicingEntitiesQuery.isLoading && tenant) {
      setSelectedEntity({id: defaultEntity.id, tenantId: tenant.id})
    }
  }, [selectedEntity, defaultEntity, setSelectedEntity, listInvoicingEntitiesQuery.isLoading])
  
  // Validate that selected entity exists in current tenant
  const isSelectedValid = selectedEntity && entities.some(e => e.id === selectedEntity.id)
  const finalSelectedId = isSelectedValid ? selectedEntity.id : defaultEntity?.id

  return {
    selectedEntityId: finalSelectedId,
    setSelectedEntityId: (id: string | undefined) => {
      if (tenant) {
        setSelectedEntity(id ? {id, tenantId: tenant.id} : undefined)
      }
    },

    entities,
    currentEntity,
    defaultEntity,
    isLoading: listInvoicingEntitiesQuery.isLoading,
    error: listInvoicingEntitiesQuery.error,
  }
}