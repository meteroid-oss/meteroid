import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArchiveIcon, PencilIcon, PlusIcon, TagIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import {
  archiveCreditUnit,
  listCreditUnits,
} from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { AddCreditUnitModal } from './AddCreditUnitModal'
import { CreditUnitSellingDialog } from './CreditUnitSellingDialog'
import { EditCreditUnitDialog } from './EditCreditUnitDialog'
import { invalidateCreditUnits } from './queries'

export const CreditUnitsCard = () => {
  const queryClient = useQueryClient()
  const [showAddModal, setShowAddModal] = useState(false)
  const [unitToArchive, setUnitToArchive] = useState<CreditUnit | null>(null)
  const [sellingUnitId, setSellingUnitId] = useState<string | null>(null)
  const [editingUnitId, setEditingUnitId] = useState<string | null>(null)

  const unitsQuery = useQuery(listCreditUnits, { includeArchived: true })

  const archiveMut = useMutation(archiveCreditUnit, {
    onSuccess: () => {
      invalidateCreditUnits(queryClient)
      toast.success('Credit unit archived')
      setUnitToArchive(null)
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The credit unit could not be archived')),
  })

  const units = unitsQuery.data?.units ?? []
  const sellingUnit = units.find(unit => unit.id === sellingUnitId)
  const editingUnit = units.find(unit => unit.id === editingUnitId)

  return (
    <>
      <Card className="px-8 py-6 max-w-[950px]">
        <div className="space-y-4">
          <div className="flex justify-end">
            <Button size="sm" onClick={() => setShowAddModal(true)} className="gap-2">
              <PlusIcon className="h-4 w-4" />
              Add credit unit
            </Button>
          </div>

          <div className="border rounded-lg">
            {unitsQuery.isLoading ? (
              <div className="p-8 text-center">
                <Spinner />
              </div>
            ) : units.length === 0 ? (
              <div className="p-8 text-center text-muted-foreground text-xs">
                No credit units yet. Add one to price usage in credits instead of money.
              </div>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Code</TableHead>
                    <TableHead>Name</TableHead>
                    <TableHead>Symbol</TableHead>
                    <TableHead>Precision</TableHead>
                    <TableHead>Value per credit</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {units.map(unit => (
                    <TableRow key={unit.id}>
                      <TableCell className="font-mono">{unit.code}</TableCell>
                      <TableCell>
                        {unit.name} / {unit.pluralName}
                      </TableCell>
                      <TableCell className="text-muted-foreground">{unit.symbol || '-'}</TableCell>
                      <TableCell>{unit.precision}</TableCell>
                      <TableCell>
                        <SellingSummary unit={unit} />
                      </TableCell>
                      <TableCell>
                        {unit.archived ? (
                          <Badge variant="outline" size="sm">
                            Archived
                          </Badge>
                        ) : (
                          <Badge variant="secondary" size="sm">
                            Active
                          </Badge>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={`Edit ${unit.pluralName}`}
                          onClick={() => setEditingUnitId(unit.id)}
                        >
                          <PencilIcon className="h-4 w-4" />
                        </Button>
                        {!unit.archived && (
                          <Button
                            size="sm"
                            variant="ghost"
                            aria-label={`Sell ${unit.pluralName}`}
                            onClick={() => setSellingUnitId(unit.id)}
                          >
                            <TagIcon className="h-4 w-4" />
                          </Button>
                        )}
                        {!unit.archived && (
                          <Button
                            size="sm"
                            variant="ghost"
                            aria-label={`Archive ${unit.pluralName}`}
                            onClick={() => setUnitToArchive(unit)}
                            className="text-destructive hover:text-destructive"
                          >
                            <ArchiveIcon className="h-4 w-4" />
                          </Button>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </div>
        </div>
      </Card>

      <AddCreditUnitModal
        open={showAddModal}
        onClose={() => setShowAddModal(false)}
        onSetValue={setSellingUnitId}
      />

      {sellingUnit && (
        <CreditUnitSellingDialog unit={sellingUnit} onClose={() => setSellingUnitId(null)} />
      )}

      {editingUnit && (
        <EditCreditUnitDialog
          key={editingUnit.id}
          unit={editingUnit}
          onClose={() => setEditingUnitId(null)}
        />
      )}

      <ConfirmationModal
        visible={!!unitToArchive}
        onSelectConfirm={() => unitToArchive && archiveMut.mutate({ id: unitToArchive.id })}
        onSelectCancel={() => setUnitToArchive(null)}
        header="Archive credit unit"
        danger
        buttonLabel="Archive"
      >
        Archiving blocks this unit from new prices, packs and grants. Existing ones keep working.
      </ConfirmationModal>
    </>
  )
}

const SellingSummary = ({ unit }: { unit: CreditUnit }) => {
  // An archived tax product stops every sale of the unit (`unit_wallets.rs`).
  const product = useQuery(
    getProduct,
    { productId: unit.productId ?? '' },
    { enabled: !!unit.productId }
  ).data?.product
  if (unit.prices.length === 0) {
    return <span className="text-muted-foreground">No money value</span>
  }
  return (
    <div className="flex flex-wrap items-center gap-1">
      {[...unit.prices]
        .sort((a, b) => a.currency.localeCompare(b.currency))
        .map(price => (
          <Badge key={price.id} variant="outline" size="sm" className="font-normal">
            {price.currency} {formatCurrencyNoRounding(price.unitPrice, price.currency)}
          </Badge>
        ))}
      {!unit.productId && (
        <Badge variant="warning" size="sm">
          No tax product
        </Badge>
      )}
      {product?.archivedAt && (
        <Badge variant="warning" size="sm">
          Tax product archived
        </Badge>
      )}
    </div>
  )
}
