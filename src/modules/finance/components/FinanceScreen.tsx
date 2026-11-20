import { useCallback, useState, type ReactNode } from 'react'
import { ChevronLeft, ChevronRight, MessageSquare, Plus } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { useAppDispatch } from '../../../store/hooks'
import { setRightChatPanelOpen } from '../../../features/ui/uiSlice'
import { requestChatComposerText } from '../../../services/chat/chatComposerRequests'
import { useNarrowContainer } from '../../../hooks/useNarrowContainer'
import { formatMonthTitle } from '../engines/financeFormat'
import { FinanceOverviewTab } from './FinanceOverviewTab'
import { FinancePaymentsTab } from './FinancePaymentsTab'
import { FinanceMovementsTab } from './FinanceMovementsTab'
import { FinanceSavingsTab } from './FinanceSavingsTab'
import { FinanceManageTab } from './FinanceManageTab'
import { FinanceDeveloperView } from './FinanceDeveloperView'
import { FinanceEntryProvider } from './entry/FinanceEntryProvider'
import { useFinanceEntry } from './entry/financeEntryContext'
import '../styles/finance.css'

export type FinanceTab = 'overview' | 'payments' | 'movements' | 'savings' | 'manage' | 'dev'

const TABS: Array<{ id: FinanceTab; label: string }> = [
  { id: 'overview', label: 'Resumen' },
  { id: 'payments', label: 'Pagos' },
  { id: 'movements', label: 'Movimientos' },
  { id: 'savings', label: 'Ahorro' },
  { id: 'manage', label: 'Administrar' },
]

const TITLES: Partial<Record<FinanceTab, string>> = { savings: 'Ahorro', manage: 'Administrar', dev: 'Desarrollo' }

/** Width of the view below which the phone layout applies. */
const PHONE_MAX_WIDTH = 641

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
 * `FinanceEntryProvider`) or by the assistant.
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

  const openChat = useCallback(() => {
    dispatch(setRightChatPanelOpen(true))
    requestChatComposerText(null)
  }, [dispatch])

  const hasMonth = tab === 'overview' || tab === 'payments' || tab === 'movements'
  const title = TITLES[tab] ?? formatMonthTitle(month)
  const heading = hasMonth ? <div className="finance-month">
    <button type="button" className="finance-icon-button" aria-label="Mes anterior" onClick={() => setMonth((value) => shiftMonth(value, -1))}><ChevronLeft size={18} /></button>
    <h1>{title}</h1>
    <button type="button" className="finance-icon-button" aria-label="Mes siguiente" onClick={() => setMonth((value) => shiftMonth(value, 1))}><ChevronRight size={18} /></button>
    {month !== currentMonth() && <button type="button" className="finance-link-button" onClick={() => setMonth(currentMonth())}>Hoy</button>}
  </div> : <h1>{title}</h1>

  const content: Record<FinanceTab, ReactNode> = {
    overview: <FinanceOverviewTab library={library} month={month} onTab={setTab} />,
    payments: <FinancePaymentsTab library={library} month={month} />,
    movements: <FinanceMovementsTab library={library} month={month} />,
    savings: <FinanceSavingsTab library={library} />,
    manage: <FinanceManageTab library={library} />,
    dev: <FinanceDeveloperView library={library} />,
  }

  return <main ref={onRoot} className={`notia-main finance-screen${phone ? ' finance-screen--phone' : ''}`}>
    <div className="finance-screen__wrap">
      <header className="finance-header">
        <div className="finance-header__main">
          <span className="finance-eyebrow">Finanzas</span>
          {heading}
        </div>
        <div className="finance-header__actions">
          <button type="button" className={phone ? 'finance-icon-button' : 'finance-button finance-button--ghost'} aria-label="Con el asistente" onClick={openChat}>
            <MessageSquare size={phone ? 18 : 16} aria-hidden="true" />{!phone && 'Con el asistente'}
          </button>
          <button type="button" className="finance-button finance-button--primary" onClick={() => entry?.openMenu()}><Plus size={16} aria-hidden="true" />Cargar</button>
        </div>
      </header>

      <nav className="finance-tabs" aria-label="Secciones de Finanzas">
        <div className="finance-tabs__list" role="tablist">
          {TABS.map((item) => <button key={item.id} type="button" role="tab" id={`finance-tab-${item.id}`} aria-controls="finance-tabpanel" aria-selected={tab === item.id} onClick={(event) => {
            setTab(item.id)
            // On a phone the tabs scroll sideways: keep the chosen one whole.
            event.currentTarget.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
          }}>{item.label}</button>)}
          <button type="button" role="tab" id="finance-tab-dev" aria-controls="finance-tabpanel" aria-selected={tab === 'dev'} className="finance-tabs__dev" onClick={() => setTab('dev')}>Dev</button>
        </div>
      </nav>

      <section id="finance-tabpanel" role="tabpanel" aria-labelledby={`finance-tab-${tab}`} className="finance-tabpanel">
        {content[tab]}
      </section>
    </div>
  </main>
}
