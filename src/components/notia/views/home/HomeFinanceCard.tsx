import { useEffect, useState } from 'react'
import { CreditCard } from 'lucide-react'
import type { HomeCard, HomeFinance } from '../../../../services/home/homeTypes'
import { getDollarQuotes, type DollarQuote, type DollarQuoteKind } from '../../../../modules/finance/services/dollarQuotesService'
import { formatMoney, formatMoneyList, formatUpdatedAt } from '../../../../modules/finance/engines/financeFormat'
import { HomeCardShell, HomeMoreButton } from './HomeCardShell'
import { countLabel } from './homeDisplay'

const QUOTE_KINDS: Array<{ kind: DollarQuoteKind; name: string }> = [
  { kind: 'oficial', name: 'Oficial' },
  { kind: 'blue', name: 'Blue' },
  { kind: 'tarjeta', name: 'Tarjeta' },
]

interface HomeFinanceCardProps {
  card: HomeCard<HomeFinance>
  onOpenFinance: () => void
  /** Puts `prompt` in the side chat's composer; `null` only focuses it. */
  onOpenChat: (prompt: string | null) => void
}

function quoteValue(quote: DollarQuote | undefined): string {
  return quote ? quote.sell.toLocaleString('es-AR', { maximumFractionDigits: 0 }) : '—'
}

/** The month of Finanzas: expenses, what is left, what is still to pay and the dollar. */
export function HomeFinanceCard({ card, onOpenFinance, onOpenChat }: HomeFinanceCardProps) {
  const finance = card.data
  const [quotes, setQuotes] = useState<DollarQuote[] | null>(null)
  const [quotesFailed, setQuotesFailed] = useState(false)

  useEffect(() => {
    let isCurrent = true
    getDollarQuotes()
      .then((next) => { if (isCurrent) setQuotes(next) })
      .catch(() => { if (isCurrent) setQuotesFailed(true) })
    return () => { isCurrent = false }
  }, [])

  const oficial = quotes?.find((quote) => quote.kind === 'oficial')
  const quotesNote = oficial ? formatUpdatedAt(oficial.updatedAt).replace(', ', ' · ') : quotesFailed ? 'Sin conexión' : ''

  return (
    <HomeCardShell
      id="home-finance-title"
      title="Finanzas"
      icon={<CreditCard size={15} strokeWidth={1.75} />}
      className="home-card--finance"
      error={card.error}
      action={<HomeMoreButton label="Abrir" ariaLabel="Abrir Finanzas" onClick={onOpenFinance} />}
    >
      {finance ? (
        <>
          <div className="home-finance__month">
            <span className="home-card__sub">Gastos de {finance.monthLabel}</span>
            <span className="home-number">{finance.expenses.length > 0 ? formatMoneyList(finance.expenses, 0) : 'Sin gastos'}</span>
            <span className="home-card__sub">{countLabel(finance.expenseCount, 'gasto', 'gastos')}</span>
          </div>
          <div className="home-finance__pair">
            <div className="home-finance__figure">
              <span className="home-card__sub">Te queda</span>
              <span className="home-finance__value home-finance__value--saved">
                {finance.income.length > 0 ? finance.left.map((money) => formatMoney(money, 0)).join(' · ') || '$ 0' : '—'}
              </span>
            </div>
            <div className="home-finance__figure">
              <span className="home-card__sub">Falta pagar</span>
              <span className="home-finance__value">
                {finance.pendingCount === 0 ? 'Nada' : [finance.pending.map((money) => formatMoney(money, 0)).join(' · '), finance.pendingNote].filter(Boolean).join(' + ')}
              </span>
            </div>
          </div>
          <div className="home-finance__quotes">
            <div className="home-list-head">
              <span className="home-label">Dólar venta</span>
              <span className="home-mono-note">{quotesNote}</span>
            </div>
            <div className="home-fx">
              {QUOTE_KINDS.map(({ kind, name }) => (
                <div key={kind}>
                  <span>{name}</span>
                  <b>{quoteValue(quotes?.find((quote) => quote.kind === kind))}</b>
                </div>
              ))}
            </div>
          </div>
          <div className="home-spacer" />
          <button type="button" className="home-button home-finance__review" onClick={() => onOpenChat(null)}>
            <span>{finance.pendingCount > 0 ? `${countLabel(finance.pendingCount, 'pago', 'pagos')} del mes por hacer` : 'Pagos del mes al día'}</span>
            <span className="home-finance__review-action">Cargar con el asistente</span>
          </button>
        </>
      ) : null}
    </HomeCardShell>
  )
}
