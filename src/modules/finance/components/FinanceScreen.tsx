import { useCallback, useState, type ReactNode } from 'react'
import { ChevronLeft, ChevronRight, MessageSquare, Plus, RefreshCw } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { useAppDispatch } from '../../../store/hooks'
import { setRightChatPanelOpen } from '../../../features/ui/uiSlice'
import { requestChatComposerText } from '../../../services/chat/chatComposerRequests'
import { useNarrowContainer } from '../../../hooks/useNarrowContainer'
import { getDollarQuotes } from '../services/dollarQuotesService'
import { notifyFinanceDataChanged } from '../services/financeDataEvents'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatMonthTitle, formatUpdatedAt } from '../engines/financeFormat'
import { FinanceOverviewTab } from './FinanceOverviewTab'
import { FinanceMovementsTab } from './FinanceMovementsTab'
import { FinanceSalaryTab } from './FinanceSalaryTab'
import { FinanceProductsTab } from './FinanceProductsTab'
import { FinanceDeveloperView } from './FinanceDeveloperView'
import { FinanceManageTab } from './FinanceManageTab'
import { FinanceEntryProvider } from './entry/FinanceEntryProvider'
import { useFinanceEntry } from './entry/financeEntryContext'
import '../styles/finance.css'

type FinanceTab = 'overview' | 'movements' | 'salary' | 'products' | 'manage' | 'dev'

const TABS: Array<{ id: FinanceTab; label: string; phone: string }> = [
  { id: 'overview', label: 'Resumen', phone: 'Resumen' },
  { id: 'movements', label: 'Movimientos', phone: 'Movimientos' },
  { id: 'salary', label: 'Sueldo y ahorro', phone: 'Sueldo y ahorro' },
  { id: 'products', label: 'Productos y tickets', phone: 'Productos' },
  { id: 'manage', label: 'Administrar', phone: 'Administrar' },
]

/** Width of the view below which the phone boards of the canvas apply. */
const PHONE_MAX_WIDTH = 641

const TICKET_PROMPT = 'Te mando un ticket para cargar en Finanzas.'

function currentMonth(): string {
  const today = new Date()
  return `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}`
}

function shiftMonth(month: string, offset: number): string {
  const [year, monthNumber] = month.split('-').map(Number)
  const date = new Date(year, monthNumber - 1 + offset, 1)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`
}

/**
 * Finanzas: everything is loaded by hand (the forms Rust builds, through
 * `FinanceEntryProvider`) or by the assistant. In the space of a phone it
 * follows the phone boards of the canvas.
 */
export function FinanceScreen({ library }: { library: NotiaLibrary }) {
  const [root, setRoot] = useState<HTMLElement | null>(null)
  const phone = useNarrowContainer(root, PHONE_MAX_WIDTH)
  return <FinanceEntryProvider library={library} phone={phone}>
    <FinanceScreenView library={library} phone={phone} onRoot={setRoot} />
  </FinanceEntryProvider>
}

function FinanceScreenView({ library, phone, onRoot }: { library: NotiaLibrary; phone: boolean; onRoot: (element: HTMLElement | null) => void }) {
  const dispatch = useAppDispatch()
  const entry = useFinanceEntry()
  const [tab, setTab] = useState<FinanceTab>('overview')
  const [month, setMonth] = useState(currentMonth)
  const quotes = useFinanceResource(getDollarQuotes, 'No se pudieron cargar las cotizaciones.')

  const openChat = useCallback((prompt: string | null) => {
    dispatch(setRightChatPanelOpen(true))
    requestChatComposerText(prompt)
  }, [dispatch])
  const refresh = useCallback(() => notifyFinanceDataChanged(), [])
  const showMovements = useCallback(() => setTab('movements'), [])

  const hasMonth = tab === 'overview' || tab === 'movements'
  const title = tab === 'salary' ? 'Sueldo y ahorro' : tab === 'products' ? 'Productos y tickets' : tab === 'manage' ? 'Administrar' : tab === 'dev' ? 'Desarrollo' : formatMonthTitle(month)
  const assistant = tab === 'products'
    ? { label: 'Mandar un ticket', prompt: TICKET_PROMPT }
    : { label: 'Con el asistente', prompt: null }
  const oficial = quotes.data?.find((quote) => quote.kind === 'oficial')
  const monthButtons = (child: ReactNode) => <>
    <IconButton label="Mes anterior" onClick={() => setMonth((value) => shiftMonth(value, -1))}><ChevronLeft size={phone ? 16 : 18} /></IconButton>
    {child}
    <IconButton label="Mes siguiente" onClick={() => setMonth((value) => shiftMonth(value, 1))}><ChevronRight size={phone ? 16 : 18} /></IconButton>
  </>

  return <main ref={onRoot} className={`notia-main finance-screen${phone ? ' finance-screen--phone' : ''}`}>
    <div className="finance-screen__wrap">
      {phone ? <header className="finance-phone-head">
        <div className="finance-phone-head__main">
          <div className="finance-phone-head__top">
            <span className="finance-eyebrow">Finanzas</span>
            <div className="finance-phone-head__buttons">
              <button type="button" className="finance-phone-head__chat finance-phone-head__chat--quiet" aria-label={assistant.label} onClick={() => openChat(assistant.prompt)}><MessageSquare size={20} aria-hidden="true" /></button>
              <button type="button" className="finance-phone-head__chat" aria-label="Cargar" onClick={() => entry?.openMenu()}><Plus size={22} aria-hidden="true" /></button>
            </div>
          </div>
          {hasMonth ? <div className="finance-phone-head__month">{monthButtons(<h1>{title}</h1>)}</div> : <h1>{title}</h1>}
        </div>
      </header> : <header className="finance-header">
        <div className="finance-header__main">
          <span className="finance-eyebrow">Finanzas</span>
          <div className="finance-header__title">
            {hasMonth ? monthButtons(<h1>{title}</h1>) : <h1>{title}</h1>}
          </div>
          {tab === 'overview' && <p className="finance-header__sub">Cargalo a mano con «Cargar» o pedíselo al asistente en el chat o por Telegram.</p>}
          {tab === 'products' && <p className="finance-header__sub">Cada ticket que le mandás al asistente suma sus productos con el precio y el comercio.</p>}
        </div>
        <div className="finance-header__actions">
          {tab === 'overview' && <button type="button" className="finance-button finance-button--ghost" onClick={refresh} aria-label="Actualizar datos"><RefreshCw size={16} aria-hidden="true" />Actualizar</button>}
          <button type="button" className="finance-button finance-button--ghost" onClick={() => openChat(assistant.prompt)}><MessageSquare size={16} aria-hidden="true" />{assistant.label}</button>
          <button type="button" className="finance-button finance-button--primary" onClick={() => entry?.openMenu()}><Plus size={16} aria-hidden="true" />Cargar</button>
        </div>
      </header>}

      <nav className={phone ? 'finance-phone-tabs' : 'finance-tabs'} aria-label="Secciones de Finanzas">
        <div className="finance-tabs__list" role="tablist">
          {TABS.map((item) => <button key={item.id} type="button" role="tab" id={`finance-tab-${item.id}`} aria-controls="finance-tabpanel" aria-selected={tab === item.id} onClick={(event) => {
            setTab(item.id)
            // On a phone the tabs scroll sideways: keep the chosen one whole.
            if (phone) event.currentTarget.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
          }}>
            {phone ? item.phone : item.label}
          </button>)}
          {phone ? null : <button type="button" role="tab" id="finance-tab-dev" aria-controls="finance-tabpanel" aria-selected={tab === 'dev'} className="finance-tabs__dev" onClick={() => setTab('dev')}>Dev</button>}
        </div>
        {!phone && tab === 'overview' && quotes.data && quotes.data.length > 0 && <div className="finance-tabs__quotes" aria-label="Dólar venta">
          <span className="finance-mono-label">Dólar venta</span>
          {quotes.data.map((quote) => <span key={quote.kind}>{quote.name} <strong>{formatAmount(quote.sell, 'ARS', 0)}</strong></span>)}
          {oficial && <span>{formatUpdatedAt(oficial.updatedAt)}</span>}
        </div>}
      </nav>

      <section id="finance-tabpanel" role="tabpanel" aria-labelledby={`finance-tab-${tab}`} className="finance-tabpanel">
        {tab === 'overview' && <FinanceOverviewTab library={library} month={month} officialSell={oficial?.sell ?? null} onOpenChat={openChat} onShowMovements={showMovements} phone={phone} />}
        {tab === 'movements' && <FinanceMovementsTab library={library} month={month} onOpenChat={openChat} phone={phone} />}
        {tab === 'salary' && <FinanceSalaryTab library={library} month={month} phone={phone} />}
        {tab === 'products' && <FinanceProductsTab library={library} onOpenChat={openChat} ticketPrompt={TICKET_PROMPT} phone={phone} />}
        {tab === 'manage' && <FinanceManageTab library={library} />}
        {tab === 'dev' && <FinanceDeveloperView library={library} />}
      </section>

      {phone && <footer className="finance-phone-foot">
        {tab === 'overview' && quotes.data && quotes.data.length > 0 && <div className="finance-phone-foot__quotes">
          <span>{quotes.data.map((quote) => `${quote.name} ${formatAmount(quote.sell, 'ARS', 0)}`).join(' · ')}</span>
          {oficial && <button type="button" className="finance-link-button" onClick={() => setTab('salary')}>{formatUpdatedAt(oficial.updatedAt).split(',')[0]}</button>}
        </div>}
        <button type="button" className="finance-button finance-button--quiet" aria-pressed={tab === 'dev'} onClick={() => setTab(tab === 'dev' ? 'overview' : 'dev')}>{tab === 'dev' ? 'Volver al resumen' : 'Herramientas de desarrollo'}</button>
      </footer>}
    </div>
  </main>
}

function IconButton({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return <button type="button" className="finance-icon-button" aria-label={label} onClick={onClick}>{children}</button>
}
