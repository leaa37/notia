import { useState, type ReactNode } from 'react'
import { ChevronLeft, ChevronRight, ReceiptText, Search } from 'lucide-react'
import { formatAmount, formatMoney, formatShortDate, formatSignedPercent, plural } from '../../engines/financeFormat'
import { PRODUCT_SORTS, seriesIndex, TICKET_STATUS } from '../../engines/financeProductText'
import type { FinanceProducts, ProductDetail, ProductSort, TicketRow } from '../../types/financeScreen'
import { FinancePriceChart } from '../FinancePriceChart'
import { useFinanceEntry } from '../entry/financeEntryContext'

/*
 * Productos y tickets in the space of a phone: the canvas boards «Teléfono ·
 * Productos» (a list, and a product's detail in its place) and «Productos
 * sin datos». Search, order and every figure come from Rust
 * (`finance_products`).
 */

const PHONE_CHART_HEIGHT = 150

interface Props {
  data: FinanceProducts | null
  search: string
  sort: ProductSort
  selectedId: string | null
  /** Loading or error, from the tab. */
  status: ReactNode
  onSearch: (search: string) => void
  onSort: (sort: ProductSort) => void
  onSelect: (productId: string) => void
  onOpenChat: (prompt: string) => void
  onSendTicket: () => void
}

export function PhoneProducts({ data, search, sort, selectedId, status, onSearch, onSort, onSelect, onOpenChat, onSendTicket }: Props) {
  const [showingDetail, setShowingDetail] = useState(false)
  if (!data) return <div className="finance-phone">{status}</div>
  if (data.totalCount === 0) return <PhoneProductsEmpty tickets={data.tickets} onSendTicket={onSendTicket} />
  if (showingDetail) {
    const detail = data.selected && data.selected.id === selectedId ? data.selected : null
    return <div className="finance-phone">
      <button type="button" className="finance-phone-back" onClick={() => setShowingDetail(false)}><ChevronLeft size={16} aria-hidden="true" />Productos</button>
      {detail ? <PhoneProductDetail detail={detail} onOpenChat={onOpenChat} /> : status ?? <p className="finance-status" role="status">Cargando el producto…</p>}
    </div>
  }
  return <div className="finance-phone">
    {status}
    <label className="finance-phone-search">
      <Search size={16} aria-hidden="true" />
      <span className="finance-visually-hidden">Buscar producto</span>
      <input type="search" value={search} onChange={(event) => onSearch(event.target.value)} placeholder="Leche, yerba…" />
    </label>
    <div className="finance-phone-chips finance-phone-chips--wrap" role="group" aria-label="Ordenar productos">
      {PRODUCT_SORTS.map((option) => <button key={option.id} type="button" className="finance-phone-chip" aria-pressed={sort === option.id} onClick={() => onSort(option.id)}>{option.label}</button>)}
    </div>
    <section className="finance-phone-card finance-phone-card--rows" aria-label="Productos">
      {data.products.map((product) => <button key={product.id} type="button" className="finance-phone-product" onClick={(event) => {
        onSelect(product.id)
        setShowingDetail(true)
        // The detail takes the place of the list: start it from the top.
        event.currentTarget.closest('.finance-screen')?.scrollTo?.({ top: 0 })
      }}>
        <span className="finance-phone-line__main">
          <span className="finance-phone-product__name">{product.name}</span>
          <span className="finance-phone-sub">{plural(product.merchantCount, 'comercio')} · {formatShortDate(product.lastDate)}</span>
        </span>
        <span className="finance-phone-product__end">
          <span className="finance-phone-line__end">
            <strong className="finance-phone-nowrap">{product.merchantCount > 1 ? 'desde ' : ''}{formatMoney(product.fromPrice, 0)}</strong>
            {product.changePercent !== null && <span className={product.changePercent > 0 ? 'finance-phone-sub finance-warn-text' : 'finance-phone-sub finance-teal'}>{formatSignedPercent(product.changePercent)}</span>}
          </span>
          <ChevronRight size={16} aria-hidden="true" className="finance-muted" />
        </span>
      </button>)}
      {data.products.length === 0 && <p className="finance-phone-empty-row">Ningún producto coincide.</p>}
    </section>
    <PhoneTickets tickets={data.tickets} />
  </div>
}

function PhoneProductDetail({ detail, onOpenChat }: { detail: ProductDetail; onOpenChat: (prompt: string) => void }) {
  const { best, worst } = detail
  return <>
    <div className="finance-phone-stack">
      <h2 className="finance-phone-h2--large">{detail.name}</h2>
      <span className="finance-phone-sub">{detail.aliases.length > 0 ? `También figura como ${detail.aliases.join(' y ')}` : 'Sin otros nombres'}</span>
    </div>
    <div className="finance-phone-best">
      <span className="finance-phone-line__main">
        <span className="finance-phone-best__label">Más barato hoy</span>
        <span className="finance-phone-best__where">{best.merchant} · {formatShortDate(best.date)}</span>
      </span>
      <span className="finance-phone-figure finance-phone-figure--lg">{formatAmount(best.price, detail.currency, 0)}</span>
    </div>
    <div className="finance-phone-tiles">
      <section className="finance-phone-tile">
        <span>Contra el más caro</span>
        <strong>{worst?.difference ? formatAmount(worst.difference, detail.currency, 0) : '—'}</strong>
        <small>{worst ? `${worst.merchant} · ${formatSignedPercent(worst.differencePercent ?? 0)}` : `solo en ${best.merchant}`}</small>
      </section>
      <section className="finance-phone-tile">
        <span>Desde la 1.ª compra</span>
        <strong className={detail.changePercent === null ? undefined : detail.changePercent > 0 ? 'finance-warn-text' : 'finance-figure--teal'}>{detail.changePercent === null ? '—' : formatSignedPercent(detail.changePercent)}</strong>
        <small>{detail.changeMerchant ? 'en el comercio con más compras' : 'hace falta una segunda compra'}</small>
      </section>
    </div>
    <section className="finance-phone-card" aria-label="Precio por comercio">
      <FinancePriceChart detail={detail} height={PHONE_CHART_HEIGHT} phone />
      <div className="finance-phone-merchants">
        {detail.rows.map((row) => <div key={row.merchant} className="finance-phone-line finance-phone-line--top">
          <span className="finance-phone-line__main">
            <span className="finance-phone-merchant"><i className={`finance-swatch finance-swatch--round finance-series--${seriesIndex(detail, row.merchant)}`} aria-hidden="true" />{row.merchant}</span>
            <span className="finance-phone-sub finance-phone-indent">último: {formatShortDate(row.date)}</span>
          </span>
          <span className="finance-phone-line__end">
            <strong>{formatAmount(row.price, detail.currency, 0)}</strong>
            <span className={row.cheapest ? 'finance-phone-sub finance-teal' : 'finance-phone-sub'}>{row.cheapest ? 'el más barato' : `+ ${formatAmount(row.difference ?? '0', detail.currency, 0)} (${formatSignedPercent(row.differencePercent ?? 0)})`}</span>
          </span>
        </div>)}
      </div>
    </section>
    {detail.similar && <div className="finance-phone-similar">
      <p>{detail.similar.parts.map((part, index) => part.strong ? <strong key={index}>{part.text}</strong> : <span key={index}>{part.text}</span>)}</p>
      <div className="finance-phone-actions">
        {detail.similar.actions.map((action) => <button key={action.label} type="button" className={`finance-phone-button${action.primary ? ' finance-phone-button--warn' : ''}`} onClick={() => onOpenChat(action.prompt)}>{action.label}</button>)}
      </div>
    </div>}
    <PhoneProductEdit productId={detail.id} />
    <button type="button" className="finance-phone-button" onClick={() => onOpenChat(detail.correctPrompt)}>Corregir en el chat</button>
  </>
}

function PhoneProductEdit({ productId }: { productId: string }) {
  const entry = useFinanceEntry()
  if (!entry) return null
  return <button type="button" className="finance-phone-button finance-phone-button--accent" onClick={() => entry.openEntry({ kind: 'product', id: productId })}>Renombrar o unir</button>
}

function PhoneTickets({ tickets }: { tickets: TicketRow[] }) {
  const entry = useFinanceEntry()
  return <section className="finance-phone-card finance-phone-card--list" aria-labelledby="finance-phone-tickets-title">
    <div className="finance-phone-stack">
      <h2 id="finance-phone-tickets-title">Tickets</h2>
      <span className="finance-phone-sub">{tickets.length === 0 ? 'Sin tickets registrados.' : 'Con tarjeta cuentan cuando llega el resumen'}</span>
    </div>
    {tickets.map((ticket) => <button key={ticket.id} type="button" className="finance-phone-line finance-phone-line--button" aria-label={`Editar el ticket de ${ticket.merchant}`} onClick={() => entry?.openEntry({ kind: 'purchase', id: ticket.id })}>
      <span className="finance-phone-line__main">
        <span className="finance-phone-medium">{ticket.merchant} <span className="finance-muted finance-phone-normal">· {plural(ticket.itemCount, 'producto')}</span></span>
        <span className="finance-phone-sub">{formatShortDate(ticket.date)} · {ticket.payment}</span>
      </span>
      <span className="finance-phone-line__end">
        <strong className="finance-phone-nowrap">{formatMoney(ticket.total, 0)}</strong>
        <span className={`finance-pill finance-pill--${ticket.status} finance-phone-pill`}>{TICKET_STATUS[ticket.status]}</span>
      </span>
    </button>)}
  </section>
}

function PhoneProductsEmpty({ tickets, onSendTicket }: { tickets: TicketRow[]; onSendTicket: () => void }) {
  const steps = [
    { title: 'Cargás el ticket', text: 'a mano con «+», o por el chat o Telegram.' },
    { title: 'Lo confirmás', text: ': el asistente te muestra lo que leyó.' },
    { title: 'Ves los precios', text: 'y dónde conviene comprar.' },
  ]
  return <div className="finance-phone">
    <section className="finance-phone-card finance-phone-empty-state">
      <div className="finance-phone-empty-state__icon" aria-hidden="true"><ReceiptText size={30} strokeWidth={1.8} /></div>
      <div className="finance-phone-stack">
        <h2 className="finance-phone-h2--medium">Todavía no hay productos</h2>
        <p>Mandale al asistente una foto o el PDF de un ticket. Acá vas a ver cada producto con su precio por comercio.</p>
      </div>
      <ol className="finance-phone-steps">
        {steps.map((step, index) => <li key={step.title}><span className="finance-teal">{index + 1}</span><span><strong>{step.title}</strong>{step.text.startsWith(':') ? '' : ' '}{step.text}</span></li>)}
      </ol>
      <button type="button" className="finance-phone-primary" onClick={onSendTicket}>Mandar un ticket</button>
    </section>
    <PhoneTickets tickets={tickets} />
  </div>
}
