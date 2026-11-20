import { useCallback, useState } from 'react'
import { ChevronDown, ChevronUp, Plus } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinancePayments } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatExact, formatMoneyList, formatShortDate, formatShortMonthYear, plural } from '../engines/financeFormat'
import type { CardMonth, FinancePayments, LoanMonth, ServicePayment } from '../types/financeScreen'
import { useFinanceEntry, type FinanceEntryApi } from './entry/financeEntryContext'
import { FinanceBar, FinanceCard, FinancePill, FinanceStatus, MoneyText, PendingText } from './FinanceParts'

/**
 * «Pagos»: what is paid each month. A card installment is paid with one
 * touch (and «Deshacer»); a service or a loan installment asks its amount.
 */
export function FinancePaymentsTab({ library, month }: { library: NotiaLibrary; month: string }) {
  const load = useCallback(() => getFinancePayments(library, month), [library, month])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudieron leer los pagos.')
  const entry = useFinanceEntry()
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  return <div className="finance-payments">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    <div className="finance-split finance-card finance-payments__summary">
      <div><span className="finance-mono-label">Pagado</span><strong className="finance-figure"><MoneyText list={data.paid} empty="$ 0" /></strong></div>
      <div><span className="finance-mono-label">Falta pagar</span><strong className={`finance-figure${data.pending.length > 0 || data.pendingNote ? ' finance-warn-text' : ''}`}><PendingText pending={data.pending} note={data.pendingNote} /></strong></div>
    </div>
    <Services data={data} entry={entry} />
    <Cards data={data} entry={entry} />
    <Loans data={data} entry={entry} />
  </div>
}

function AddButton({ label, onClick }: { label: string; onClick: () => void }) {
  return <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={onClick}><Plus size={14} aria-hidden="true" />{label}</button>
}

function Services({ data, entry }: { data: FinancePayments; entry: FinanceEntryApi | null }) {
  const pay = (service: ServicePayment) => entry?.openEntry({ kind: 'service-payment', prefill: { serviceId: service.id, month: data.month } })
  return <FinanceCard title="Servicios" sub={data.services.length > 0 ? `${data.services.filter((service) => service.paid).length} de ${data.services.length} pagados` : undefined} action={<AddButton label="Servicio" onClick={() => entry?.openEntry({ kind: 'service' })} />}>
    {data.services.length === 0 ? <p className="finance-empty">Sin servicios. Agregá los que pagás cada mes.</p> : <ul className="finance-pay-list" role="list">
      {data.services.map((service) => <li key={service.id} className="finance-pay-row">
        <span className="finance-pay-row__main">
          <span className="finance-pay-row__name">{service.name}</span>
          <span className="finance-pay-row__sub">{service.paid ? `Pagado el ${formatShortDate(service.paid.date)}` : 'Sin pagar'}</span>
        </span>
        <span className="finance-pay-row__amount">{service.paid ? formatExact(service.paid.amount, service.currency ?? 'ARS') : ''}</span>
        {service.paid
          ? <button type="button" className="finance-pill-button" onClick={() => entry?.openEntry({ kind: 'movement', id: service.paid?.movementId })}><FinancePill tone="paid">Pagado</FinancePill></button>
          : <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => pay(service)}>Pagar</button>}
      </li>)}
    </ul>}
  </FinanceCard>
}

function Cards({ data, entry }: { data: FinancePayments; entry: FinanceEntryApi | null }) {
  if (data.cards.length === 0) {
    return <FinanceCard title="Tarjetas" action={<AddButton label="Tarjeta" onClick={() => entry?.openEntry({ kind: 'card' })} />}>
      <p className="finance-empty">Sin tarjetas. Agregá una y cargale las compras en cuotas: cada mes ves acá qué cuotas vencen.</p>
    </FinanceCard>
  }
  return <>{data.cards.map((card) => <CardSection key={card.id} card={card} month={data.month} entry={entry} />)}</>
}

function CardSection({ card, month, entry }: { card: CardMonth; month: string; entry: FinanceEntryApi | null }) {
  const [showPurchases, setShowPurchases] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const quick = async (key: string, values: Record<string, string | string[]>) => {
    setBusy(key)
    await entry?.quickApply('card-payment', values)
    setBusy(null)
  }
  const active = card.purchases.filter((purchase) => !purchase.finished)
  return <FinanceCard
    title={card.name}
    sub={card.installments.length === 0 ? `Nada que pagar en ${formatShortMonthYear(month)}` : `Resumen de ${formatShortMonthYear(month)}: ${formatMoneyList(card.total, 0)}${card.debt.length > 0 ? ` · debe en total ${formatMoneyList(card.debt, 0)}` : ''}`}
    action={<AddButton label="Compra" onClick={() => entry?.openEntry({ kind: 'card-purchase', prefill: { cardId: card.id } })} />}
  >
    {card.installments.length > 0 && <ul className="finance-pay-list" role="list">
      {card.installments.map((installment) => {
        const key = `${installment.purchaseId}:${installment.number}`
        return <li key={key} className="finance-pay-row">
          <span className="finance-pay-row__main">
            <span className="finance-pay-row__name">{installment.name}</span>
            <span className={`finance-pay-row__sub${installment.late ? ' finance-late-text' : ''}`}>
              {installment.count === 1 ? 'En 1 pago' : `Cuota ${installment.number} de ${installment.count}`}
              {installment.late ? ` · atrasada (${formatShortMonthYear(installment.month)})` : ''}
              {installment.paid ? ` · pagada el ${formatShortDate(installment.paid.date)}` : ''}
            </span>
          </span>
          <span className="finance-pay-row__amount">{formatExact(installment.amount, installment.currency)}</span>
          {installment.paid
            ? <button type="button" className="finance-pill-button" onClick={() => entry?.openEntry({ kind: 'movement', id: installment.paid?.movementId })}><FinancePill tone="paid">Pagada</FinancePill></button>
            : <button type="button" className="finance-button finance-button--outline finance-button--small" disabled={busy !== null} onClick={() => void quick(key, { cardId: card.id, installments: [key] })}>{busy === key ? '…' : 'Pagar'}</button>}
        </li>
      })}
    </ul>}
    {card.pendingCount > 1 && <button type="button" className="finance-button finance-button--primary" disabled={busy !== null} onClick={() => void quick('all', { cardId: card.id, month })}>
      {busy === 'all' ? 'Pagando…' : `Pagar las ${card.pendingCount} cuotas · ${formatMoneyList(card.pending, 0)}`}
    </button>}
    {card.purchases.length > 0 && <div className="finance-purchases">
      <button type="button" className="finance-link-button finance-purchases__toggle" aria-expanded={showPurchases} onClick={() => setShowPurchases((value) => !value)}>
        {showPurchases ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
        {plural(active.length, 'compra en curso', 'compras en curso')}
      </button>
      {showPurchases && <ul className="finance-pay-list" role="list">
        {card.purchases.map((purchase) => <li key={purchase.id}>
          <button type="button" className="finance-purchase" onClick={() => entry?.openEntry({ kind: 'card-purchase', id: purchase.id })}>
            <span className="finance-pay-row__main">
              <span className="finance-pay-row__name">{purchase.name}</span>
              <span className="finance-pay-row__sub">
                {purchase.finished ? 'Terminada' : `${purchase.paidCount}/${purchase.count} cuotas · resta ${formatAmount(purchase.remaining, purchase.currency, 0)} · hasta ${formatShortMonthYear(purchase.lastMonth)}`}
                {purchase.category ? ` · ${purchase.category}` : ''}
              </span>
              <FinanceBar percent={(purchase.paidCount * 100) / purchase.count} tone={purchase.finished ? 'muted' : 'cards'} />
            </span>
            <span className="finance-pay-row__amount">{formatAmount(purchase.total, purchase.currency, 0)}</span>
          </button>
        </li>)}
      </ul>}
    </div>}
  </FinanceCard>
}

function Loans({ data, entry }: { data: FinancePayments; entry: FinanceEntryApi | null }) {
  return <FinanceCard title="Préstamos" action={<AddButton label="Préstamo" onClick={() => entry?.openEntry({ kind: 'loan' })} />}>
    {data.loans.length === 0 ? <p className="finance-empty">Sin préstamos.</p> : <ul className="finance-pay-list" role="list">
      {data.loans.map((loan) => <LoanRow key={loan.id} loan={loan} entry={entry} />)}
    </ul>}
  </FinanceCard>
}

function LoanRow({ loan, entry }: { loan: LoanMonth; entry: FinanceEntryApi | null }) {
  const sub = loan.finished
    ? `Pagado entero: ${formatAmount(loan.totalPaid, loan.currency, 0)}`
    : loan.number === null
      ? `Próxima: cuota ${loan.nextNumber} de ${loan.count}`
      : `Cuota ${loan.number} de ${loan.count}${loan.paid ? ` · pagada el ${formatShortDate(loan.paid.date)}` : ''}${loan.lateCount > 0 ? ` · ${plural(loan.lateCount, 'atrasada', 'atrasadas')}` : ''}`
  // Only what was paid: the next installment costs what it costs.
  const amount = loan.paid?.amount
  return <li className="finance-pay-row finance-pay-row--loan">
    <span className="finance-pay-row__main">
      <span className="finance-pay-row__name">{loan.name}</span>
      <span className={`finance-pay-row__sub${loan.lateCount > 0 ? ' finance-late-text' : ''}`}>{sub}</span>
      <FinanceBar percent={(loan.paidCount * 100) / loan.count} label={`${loan.paidCount} de ${loan.count} cuotas`} />
      <span className="finance-pay-row__sub">{loan.paidCount}/{loan.count} pagadas · {formatAmount(loan.totalPaid, loan.currency, 0)} de {formatAmount(loan.amount, loan.currency, 0)} prestados</span>
    </span>
    <span className="finance-pay-row__amount">{amount ? formatExact(amount, loan.currency) : ''}</span>
    {loan.paid || loan.finished
      ? <FinancePill tone="paid">{loan.finished ? 'Terminado' : 'Pagada'}</FinancePill>
      : <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry?.openEntry({ kind: 'loan-payment', prefill: { loanId: loan.id } })}>Pagar cuota</button>}
  </li>
}
