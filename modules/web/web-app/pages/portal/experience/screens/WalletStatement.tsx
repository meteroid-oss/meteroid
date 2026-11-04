import { useMutation } from '@connectrpc/connect-query'
import { Download } from 'lucide-react'
import { useState } from 'react'

import { LEDGER_PAGE_SIZE, collectLedger, downloadLedgerCsv } from '@/features/credit/ledgerExport'
import { LedgerEntryType } from '@/rpc/portal/wallets/v1/models_pb'
import { listLedger } from '@/rpc/portal/wallets/v1/wallets-PortalWalletsService_connectquery'

import { useWalletLedger } from '../hooks'
import { Mono, PButton, Pager, PanelCard, Spinner } from '../primitives'
import { day, walletAmount } from '../walletFormat'

import type { Wallet } from '@/rpc/portal/wallets/v1/models_pb'
import type { CSSProperties } from 'react'

const MOVEMENT_LABEL: Record<LedgerEntryType, string> = {
  [LedgerEntryType.UNSPECIFIED]: 'Movement',
  [LedgerEntryType.GRANTED]: 'Credit added',
  [LedgerEntryType.APPLIED]: 'Applied to invoice',
  [LedgerEntryType.RETURNED]: 'Credit returned',
  [LedgerEntryType.REMOVED]: 'Credit removed',
  [LedgerEntryType.EXPIRED]: 'Credit expired',
  [LedgerEntryType.ROLLED_OVER]: 'Rolled over',
}

const headStyle: CSSProperties = {
  padding: '9px 20px',
  fontSize: 11.5,
  fontWeight: 600,
  color: 'var(--mtp-text-3)',
  textAlign: 'left',
  borderBottom: '1px solid var(--mtp-border)',
}

const visuallyHidden: CSSProperties = {
  position: 'absolute',
  width: 1,
  height: 1,
  overflow: 'hidden',
  clip: 'rect(0 0 0 0)',
  whiteSpace: 'nowrap',
}

const cellStyle: CSSProperties = {
  padding: '11px 20px',
  borderTop: '1px solid var(--mtp-border)',
  verticalAlign: 'top',
}

export const WalletStatement = ({ wallet, named }: { wallet: Wallet; named: boolean }) => {
  const [page, setPage] = useState(0)
  const ledger = useWalletLedger(wallet.id, page, LEDGER_PAGE_SIZE)
  const fetchPage = useMutation(listLedger)
  const [exporting, setExporting] = useState(false)
  const [exportFailed, setExportFailed] = useState(false)
  const entries = ledger.data?.entries ?? []
  const totalPages = ledger.data?.paginationMeta?.totalPages ?? 0
  const title = wallet.creditUnit ? wallet.creditUnit.pluralName : 'Credit'

  const exportCsv = async () => {
    setExporting(true)
    setExportFailed(false)
    try {
      const all = await collectLedger(async (pageIndex, perPage) => {
        const response = await fetchPage.mutateAsync({
          walletId: wallet.id,
          pagination: { page: pageIndex, perPage },
        })
        return { entries: response.entries, totalPages: response.paginationMeta?.totalPages ?? 0 }
      })
      const slug = title.toLowerCase().replace(/\s+/g, '-')
      downloadLedgerCsv(
        wallet.creditUnit
          ? `${slug}-statement.csv`
          : `${slug}-${wallet.currency.toLowerCase()}-statement.csv`,
        wallet.creditUnit?.pluralName ?? wallet.currency,
        all.map(entry => ({
          date: entry.effectiveAt,
          movement: MOVEMENT_LABEL[entry.type],
          description: entry.description,
          amount: entry.amount,
          balanceAfter: entry.balanceAfter,
          invoiceId: entry.invoiceId,
          creditNoteId: entry.creditNoteId,
        })),
        wallet.creditUnit?.precision
      )
    } catch {
      setExportFailed(true)
    } finally {
      setExporting(false)
    }
  }

  return (
    <PanelCard
      title={
        named
          ? wallet.creditUnit
            ? `${title} statement`
            : `${title} statement · ${wallet.currency}`
          : 'Statement'
      }
      action={
        entries.length > 0 ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {exportFailed && (
              <span role="alert" style={{ fontSize: 12, color: 'var(--mtp-danger)' }}>
                Download failed, please retry.
              </span>
            )}
            <PButton variant="ghost" size="sm" loading={exporting} onClick={() => void exportCsv()}>
              <Download size={13} /> Export CSV
            </PButton>
          </div>
        ) : undefined
      }
    >
      {ledger.isLoading ? (
        <div style={{ display: 'flex', justifyContent: 'center', padding: 20 }}>
          <Spinner size={16} />
        </div>
      ) : entries.length === 0 ? (
        <div style={{ padding: '14px 20px', fontSize: 13, color: 'var(--mtp-text-3)' }}>
          No movements yet.
        </div>
      ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
          <thead>
            <tr>
              <th scope="col" style={headStyle}>
                Date
              </th>
              <th scope="col" style={headStyle}>
                Description
              </th>
              <th scope="col" style={{ ...headStyle, textAlign: 'right' }}>
                Amount
              </th>
              <th scope="col" className="mtp-hide-sm" style={{ ...headStyle, textAlign: 'right' }}>
                Credit on account
              </th>
            </tr>
          </thead>
          <tbody>
            {entries.map(entry => {
              const added = entry.amount >= 0n
              const magnitude = walletAmount(
                added ? entry.amount : -entry.amount,
                wallet.currency,
                wallet.creditUnit
              )
              return (
                <tr key={entry.id}>
                  <td
                    style={{
                      ...cellStyle,
                      whiteSpace: 'nowrap',
                      fontSize: 12.5,
                      color: 'var(--mtp-text-2)',
                    }}
                  >
                    {day(entry.effectiveAt)}
                  </td>
                  <td style={cellStyle}>{entry.description}</td>
                  <td style={{ ...cellStyle, textAlign: 'right', whiteSpace: 'nowrap' }}>
                    <Mono
                      style={{
                        fontWeight: 500,
                        color: added ? 'var(--mtp-ok-text)' : 'var(--mtp-text)',
                      }}
                    >
                      <span style={visuallyHidden}>{added ? 'Added' : 'Deducted'} </span>
                      {added ? '+' : '−'}
                      {magnitude}
                    </Mono>
                  </td>
                  <td
                    className="mtp-hide-sm"
                    style={{ ...cellStyle, textAlign: 'right', whiteSpace: 'nowrap' }}
                  >
                    <Mono style={{ fontSize: 12.5, color: 'var(--mtp-text-3)' }}>
                      {walletAmount(entry.balanceAfter, wallet.currency, wallet.creditUnit)}
                    </Mono>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      )}
      {totalPages > 1 && (
        <Pager
          page={page + 1}
          totalPages={totalPages}
          hasPrev={page > 0}
          hasNext={page + 1 < totalPages}
          onPrev={() => setPage(current => Math.max(0, current - 1))}
          onNext={() => setPage(current => current + 1)}
        />
      )}
    </PanelCard>
  )
}
