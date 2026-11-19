import { useCallback, useEffect, useState } from 'react'
import { MessageSquare, ReceiptText, Search } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceProducts } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatMoney, formatShortDate, formatShortMonth, formatSignedPercent, plural } from '../engines/financeFormat'
import { PRODUCT_SORTS, seriesIndex, TICKET_STATUS } from '../engines/financeProductText'
import type { FinanceProducts, ProductDetail, ProductSort, TicketRow } from '../types/financeScreen'
import { FinancePriceChart } from './FinancePriceChart'
import { FinanceReviewCard } from './FinanceReviewSection'
import { FinanceStatus } from './FinanceStatus'
import { PhoneProducts } from './phone/PhoneProducts'
import { useFinanceEntry } from './entry/financeEntryContext'

const SEARCH_DELAY_MS = 250

interface Props {
  library: NotiaLibrary
  ticketPrompt: string
  onOpenChat: (prompt: string) => void
  /** The phone board of the canvas. */
  phone: boolean
}

export function FinanceProductsTab({ library, ticketPrompt, onOpenChat, phone }: Props) {
  const [search, setSearch] = useState('')
  const [appliedSearch, setAppliedSearch] = useState('')
  const [sort, setSort] = useState<ProductSort>('recent')
  const [selectedId, setSelectedId] = useState<string | null>(null)

  useEffect(() => {
    const timer = window.setTimeout(() => setAppliedSearch(search), SEARCH_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [search])

  const load = useCallback(() => getFinanceProducts(library, { search: appliedSearch, sort, selectedId }), [appliedSearch, library, selectedId, sort])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudieron cargar los productos.')
  if (phone) {
    return <PhoneProducts
      data={data}
      search={search}
      sort={sort}
      selectedId={selectedId}
      status={!data || error ? <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} /> : null}
      onSearch={setSearch}
      onSort={setSort}
      onSelect={setSelectedId}
      onOpenChat={onOpenChat}
      onSendTicket={() => onOpenChat(ticketPrompt)}
    />
  }
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  if (data.totalCount === 0) return <ProductsEmpty data={data} onSendTicket={() => onOpenChat(ticketPrompt)} />
  return <div className="finance-products">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    <div className="finance-products__layout">
      <section className="finance-card finance-products__list" aria-label="Lista de productos">
        <label className="finance-search finance-search--elevated">
          <Search size={16} aria-hidden="true" />
          <span className="finance-visually-hidden">Buscar producto</span>
          <input type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Leche, yerba…" />
        </label>
        <div className="finance-chips" role="group" aria-label="Ordenar productos">
          {PRODUCT_SORTS.map((option) => <button key={option.id} type="button" className="finance-chip finance-chip--small" aria-pressed={sort === option.id} onClick={() => setSort(option.id)}>{option.label}</button>)}
        </div>
        <p className="finance-card__sub">{plural(data.products.length, 'producto')} con precio registrado</p>
        <div className="finance-products__items">
          {data.products.map((product) => <button key={product.id} type="button" className="finance-product" aria-pressed={data.selected?.id === product.id} onClick={() => setSelectedId(product.id)}>
            <span className="finance-product__main">
              <span className="finance-product__name">{product.name}</span>
              <span className="finance-card__sub">{plural(product.merchantCount, 'comercio')} · última compra {formatShortDate(product.lastDate)}</span>
            </span>
            <span className="finance-product__price">
              <strong>{product.merchantCount > 1 ? 'desde ' : ''}{formatMoney(product.fromPrice, 0)}</strong>
              {product.changePercent !== null && <span className={product.changePercent > 0 ? 'finance-warn-text finance-small' : 'finance-teal finance-small'}>{formatSignedPercent(product.changePercent)} desde {formatShortMonth(product.firstDate)}</span>}
            </span>
          </button>)}
          {data.products.length === 0 && <p className="finance-empty">Ningún producto coincide. Probá con otra palabra o preguntale al asistente.</p>}
        </div>
      </section>
      {data.selected && <ProductDetailCard detail={data.selected} onOpenChat={onOpenChat} />}
    </div>
    <TicketsCard tickets={data.tickets} />
  </div>
}

function ProductDetailCard({ detail, onOpenChat }: { detail: ProductDetail; onOpenChat: (prompt: string) => void }) {
  const entry = useFinanceEntry()
  const { best, worst } = detail
  return <section className="finance-card finance-product-detail" aria-label="Detalle del producto">
    <div className="finance-card__head">
      <div>
        <h2 className="finance-product-detail__title">{detail.name}</h2>
        <span className="finance-card__sub">{detail.aliases.length > 0 ? `También figura como ${detail.aliases.join(' y ')}` : 'Sin otros nombres'}</span>
      </div>
      <div className="finance-card__actions">
        {entry && <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry.openEntry({ kind: 'product', id: detail.id })}>Renombrar o unir</button>}
        <button type="button" className="finance-button finance-button--quiet-outline finance-button--small" onClick={() => onOpenChat(detail.correctPrompt)}>Corregir en el chat</button>
      </div>
    </div>
    <div className="finance-product-detail__stats">
      <div className="finance-stat finance-stat--teal">
        <span>Más barato hoy</span>
        <strong>{formatAmount(best.price, detail.currency, 0)}</strong>
        <small>{best.merchant} · {formatShortDate(best.date)}</small>
      </div>
      <div className="finance-stat">
        <span>Diferencia con el más caro</span>
        <strong>{worst?.difference ? formatAmount(worst.difference, detail.currency, 0) : '—'}</strong>
        <small>{worst ? `${worst.merchant} cobra ${formatSignedPercent(worst.differencePercent ?? 0)} más` : `Solo se compró en ${best.merchant}`}</small>
      </div>
      <div className="finance-stat">
        <span>Variación desde la primera compra</span>
        <strong className={detail.changePercent === null ? undefined : detail.changePercent > 0 ? 'finance-warn-text' : 'finance-teal'}>{detail.changePercent === null ? '—' : formatSignedPercent(detail.changePercent)}</strong>
        <small>{detail.changeMerchant ? `En ${detail.changeMerchant}, el comercio con más compras` : 'Hace falta una segunda compra'}</small>
      </div>
    </div>
    <FinancePriceChart detail={detail} />
    <div className="finance-table-scroll">
      <table className="finance-table">
        <thead><tr><th scope="col">Comercio</th><th scope="col" className="is-number">Último precio</th><th scope="col">Fecha</th><th scope="col" className="is-number">Contra el más barato</th></tr></thead>
        <tbody>{detail.rows.map((row) => <tr key={row.merchant}>
          <td><span className="finance-legend-item__label"><i className={`finance-swatch finance-series--${seriesIndex(detail, row.merchant)}`} aria-hidden="true" />{row.merchant}</span></td>
          <td className="is-number finance-strong">{formatAmount(row.price, detail.currency, 0)}</td>
          <td className="finance-muted">{formatShortDate(row.date)}</td>
          <td className={`is-number ${row.cheapest ? 'finance-teal' : 'finance-soft-text'}`}>{row.cheapest ? 'el más barato' : `+ ${formatAmount(row.difference ?? '0', detail.currency, 0)} (${formatSignedPercent(row.differencePercent ?? 0)})`}</td>
        </tr>)}</tbody>
      </table>
    </div>
    {detail.similar && <FinanceReviewCard card={detail.similar} onOpenChat={onOpenChat} tone="note" />}
  </section>
}

function TicketsCard({ tickets }: { tickets: TicketRow[] }) {
  const entry = useFinanceEntry()
  return <section className="finance-card finance-tickets" aria-labelledby="finance-tickets-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-tickets-title">Tickets</h2>
      <span className="finance-card__sub">Los pagados con tarjeta cuentan recién cuando llega el resumen.</span>
    </div>
    {tickets.length === 0
      ? <p className="finance-empty">Sin tickets registrados.</p>
      : <div className="finance-table-scroll">
        <table className="finance-table finance-tickets__table">
          <thead><tr><th scope="col">Fecha</th><th scope="col">Comercio</th><th scope="col">Productos</th><th scope="col">Pago</th><th scope="col">Estado</th><th scope="col" className="is-number">Total</th>{entry && <th scope="col"><span className="finance-visually-hidden">Editar</span></th>}</tr></thead>
          <tbody>{tickets.map((ticket) => <tr key={ticket.id}>
            <td className="finance-muted">{formatShortDate(ticket.date)}</td>
            <td>{ticket.merchant}</td>
            <td>{ticket.itemCount}</td>
            <td>{ticket.payment}</td>
            <td><span className={`finance-pill finance-pill--${ticket.status}`}>{TICKET_STATUS[ticket.status]}</span></td>
            <td className="is-number finance-strong">{formatMoney(ticket.total)}</td>
            {entry && <td><button type="button" className="finance-link-button" aria-label={`Editar el ticket de ${ticket.merchant}`} onClick={() => entry.openEntry({ kind: 'purchase', id: ticket.id })}>Editar</button></td>}
          </tr>)}</tbody>
        </table>
      </div>}
  </section>
}

function ProductsEmpty({ data, onSendTicket }: { data: FinanceProducts; onSendTicket: () => void }) {
  const steps = [
    { title: 'Cargás el ticket', text: 'A mano con «Cargar» → «Ticket», o mandándolo por el chat de Notia o por Telegram.' },
    { title: 'Lo confirmás', text: 'El asistente te muestra lo que leyó y te pregunta si un producto parecido es el mismo.' },
    { title: 'Ves los precios', text: 'Último precio por comercio e historial. Si pagaste con tarjeta, se une solo a la línea del resumen.' },
  ]
  return <div className="finance-products">
    <section className="finance-card finance-products-empty">
      <div className="finance-products-empty__icon" aria-hidden="true"><ReceiptText size={34} strokeWidth={1.8} /></div>
      <div className="finance-products-empty__copy">
        <h2>Todavía no hay productos</h2>
        <p>Mandale al asistente una foto o el PDF de un ticket. Lee cada producto con su precio y el comercio, y acá vas a poder comparar dónde conviene comprar y cómo cambian los precios.</p>
      </div>
      <ol className="finance-products-empty__steps">
        {steps.map((step, index) => <li key={step.title}><span className="finance-mono-label finance-teal">{index + 1}</span><strong>{step.title}</strong><span>{step.text}</span></li>)}
      </ol>
      <button type="button" className="finance-button finance-button--primary finance-button--large" onClick={onSendTicket}><MessageSquare size={18} aria-hidden="true" />Abrir el chat para mandar un ticket</button>
    </section>
    {data.tickets.length === 0
      ? <section className="finance-card" aria-labelledby="finance-tickets-title">
        <h2 id="finance-tickets-title">Tickets</h2>
        <p className="finance-card__sub">Sin tickets registrados. Van a aparecer acá con su comercio, forma de pago y si ya se pagaron en un resumen.</p>
      </section>
      : <TicketsCard tickets={data.tickets} />}
  </div>
}
