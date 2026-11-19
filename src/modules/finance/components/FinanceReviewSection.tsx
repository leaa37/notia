import type { ReviewCard } from '../types/financeScreen'
import { useFinanceEntry } from './entry/financeEntryContext'

interface Props {
  cards: ReviewCard[]
  onOpenChat: (prompt: string) => void
}

/**
 * «Para revisar»: an answer Rust can apply (it carries `optionId`) is
 * applied at once; any other goes to the chat composer, and nothing changes
 * until the person sends it and confirms there. The phone board has its own
 * version, one card at a time (`phone/PhoneOverview`).
 */
export function FinanceReviewSection({ cards, onOpenChat }: Props) {
  return <section className="finance-review" aria-labelledby="finance-review-title">
    <div className="finance-review__head">
      <div className="finance-review__title">
        <span className="finance-dot" aria-hidden="true" />
        <h2 id="finance-review-title">Para revisar · {cards.length}</h2>
      </div>
      <p className="finance-card__sub">Tocá la respuesta y queda resuelto; lo que no tiene respuesta fija se contesta en el chat.</p>
    </div>
    <div className="finance-review__cards">
      {cards.map((card) => <FinanceReviewCard key={card.id} card={card} onOpenChat={onOpenChat} />)}
    </div>
  </section>
}

export function FinanceReviewCard({ card, onOpenChat, tone = 'review' }: { card: ReviewCard; onOpenChat: (prompt: string) => void; tone?: 'review' | 'note' }) {
  const entry = useFinanceEntry()
  return <article className={`finance-review-card finance-review-card--${tone}`}>
    {tone === 'review' && <span className="finance-review-card__label">{card.label}</span>}
    <p className="finance-review-card__text">
      {card.parts.map((part, index) => part.strong ? <strong key={index}>{part.text}</strong> : <span key={index}>{part.text}</span>)}
    </p>
    <div className="finance-review-card__actions">
      {card.actions.map((action) => <button key={action.label} type="button" className={`finance-button ${action.primary ? 'finance-button--outline' : 'finance-button--quiet-outline'} finance-button--small`} onClick={() => {
        if (entry && action.optionId) void entry.resolve(card.id, action.optionId)
        else onOpenChat(action.prompt)
      }}>{action.label}</button>)}
    </div>
  </article>
}
