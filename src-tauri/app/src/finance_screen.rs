//! Lo que muestran las pestañas de Finanzas (web y app de Android), ya
//! calculado: la pantalla solo lo dibuja.
//!
//! - `finance_overview`: el Resumen del mes (saldo, ritmo, lo que falta
//!   pagar, categorías con presupuesto, seis meses, deudas, ahorro).
//! - `finance_payments`: Pagos del mes (servicios, tarjetas, préstamos).
//! - `finance_movements` y `finance_search`: el mes o todo el historial.
//! - `finance_savings`: las reservas, el dólar y sus movimientos.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::finance::{
    add_months, current_month, day_label, days_in_month, format_cents, month_label, month_range, month_short, read, today,
    valid_month, FinanceCommandError, FinanceCommandResult, FinanceContext, CATEGORY_SALARY,
};
use crate::finance_budgets::{self as budgets, BudgetRow};
use crate::finance_model::{self as model, cents, Category, Movement, Named, PurchaseState};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub currency: String,
    pub amount: String,
}

fn money_list(totals: &BTreeMap<String, i128>) -> Vec<Money> {
    let mut list = totals
        .iter()
        .filter(|(_, amount)| **amount != 0)
        .map(|(currency, amount)| Money { currency: currency.clone(), amount: format_cents(*amount) })
        .collect::<Vec<_>>();
    // Pesos first.
    list.sort_by_key(|money| money.currency != "ARS");
    list
}

fn check_month(month: &str) -> FinanceCommandResult<()> {
    if valid_month(month) {
        Ok(())
    } else {
        Err(FinanceCommandError::invalid("El mes no es válido."))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonthNav {
    pub month: String,
    pub label: String,
    pub previous: String,
    pub next: String,
    pub is_current: bool,
}

fn nav(month: &str) -> MonthNav {
    MonthNav {
        month: month.to_string(),
        label: month_label(month),
        previous: add_months(month, -1),
        next: add_months(month, 1),
        is_current: month == current_month(),
    }
}

// ---------------------------------------------------------------------------
// Movement rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovementRow {
    pub id: String,
    /// `expense` or `income`.
    pub kind: String,
    pub name: String,
    pub category_id: Option<String>,
    pub category: Option<String>,
    pub account: Option<String>,
    pub amount: String,
    pub currency: String,
    pub date: String,
    /// What it pays: «Visa · Hogar», «Préstamo», «Servicio de noviembre 2026».
    pub detail: Option<String>,
    /// `card`, `loan`, `service` or `salary`.
    pub tag: Option<&'static str>,
}

/// What rows need from the rest of Finanzas.
struct Names {
    categories: HashMap<String, String>,
    accounts: HashMap<String, String>,
    cards: HashMap<String, String>,
    /// The card of each purchase.
    purchases: HashMap<String, String>,
    loans: std::collections::HashSet<String>,
}

fn names(connection: &Connection) -> rusqlite::Result<Names> {
    let to_map = |list: Vec<Named>| list.into_iter().map(|item| (item.id, item.name)).collect::<HashMap<_, _>>();
    Ok(Names {
        categories: model::list_categories(connection)?.into_iter().map(|category| (category.id, category.name)).collect(),
        accounts: to_map(model::list_accounts(connection)?),
        cards: to_map(model::list_cards(connection)?),
        purchases: model::list_purchases(connection)?.into_iter().map(|purchase| (purchase.id, purchase.card_id)).collect(),
        loans: model::list_loans(connection)?.into_iter().map(|loan| loan.id).collect(),
    })
}

fn row(movement: &Movement, names: &Names) -> MovementRow {
    let category = movement.category_id.as_ref().and_then(|id| names.categories.get(id)).cloned();
    // The name of a payment already says which installment it is.
    let (detail, tag) = if let Some(card_id) = movement.card_purchase_id.as_ref().and_then(|id| names.purchases.get(id)) {
        let card = names.cards.get(card_id).cloned().unwrap_or_default();
        (Some(category.as_ref().map_or(card.clone(), |category| format!("{card} · {category}"))), Some("card"))
    } else if movement.loan_id.as_ref().is_some_and(|id| names.loans.contains(id)) {
        (Some("Préstamo".to_string()), Some("loan"))
    } else if movement.service_id.is_some() {
        (movement.service_month.as_deref().map(|month| format!("Servicio de {}", month_label(month))), Some("service"))
    } else if movement.kind == "income" && movement.category_id.as_deref() == Some(CATEGORY_SALARY.0) {
        (None, Some("salary"))
    } else {
        (None, None)
    };
    MovementRow {
        id: movement.id.clone(),
        kind: movement.kind.clone(),
        name: movement.name.clone(),
        category_id: movement.category_id.clone(),
        category,
        account: movement.account_id.as_ref().and_then(|id| names.accounts.get(id)).cloned(),
        amount: movement.amount.clone(),
        currency: movement.currency.clone(),
        date: movement.date.clone(),
        detail,
        tag,
    }
}

fn month_movements(connection: &Connection, month: &str) -> FinanceCommandResult<Vec<Movement>> {
    let (from, to) = month_range(month).ok_or_else(|| FinanceCommandError::invalid("El mes no es válido."))?;
    Ok(model::movements_between(connection, &from, &to)?)
}

fn totals_of<'a>(movements: impl Iterator<Item = &'a Movement>) -> BTreeMap<String, i128> {
    let mut totals = BTreeMap::new();
    for movement in movements {
        *totals.entry(movement.currency.clone()).or_insert(0) += cents(&movement.amount);
    }
    totals
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthPayload {
    pub context: FinanceContext,
    pub month: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub currency: String,
    pub income: String,
    pub salary: String,
    pub other_income: String,
    pub expenses: String,
    /// What went to savings, net of what came back.
    pub saved: String,
    /// Income minus expenses minus savings.
    pub left: String,
    /// Share of the income that was spent.
    pub spent_percent: Option<f64>,
    /// Share of the income that was saved.
    pub saved_percent: Option<f64>,
    pub previous_expenses: String,
    /// How much more (or less) was spent than the month before, in percent.
    pub expenses_change: Option<f64>,
}

/// How the month's spending goes, in pesos.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pace {
    pub days_elapsed: u32,
    pub days_in_month: u32,
    pub daily_average: String,
    /// At this pace, what the month ends spending (only the current month).
    pub projected: Option<String>,
    pub expense_count: usize,
    pub biggest: Option<MovementRow>,
}

/// Something to pay in the month.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commitment {
    /// `service`, `card` or `loan`.
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub amount: Option<String>,
    pub currency: String,
    /// `paid`, `pending` or `late`.
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commitments {
    pub items: Vec<Commitment>,
    pub paid: Vec<Money>,
    /// What is still to pay and is known exactly (card installments).
    pub pending: Vec<Money>,
    /// What is still to pay without an amount yet: «y 2 servicios».
    pub pending_note: Option<String>,
    pub paid_count: usize,
    pub pending_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRow {
    pub category_id: Option<String>,
    pub name: String,
    pub currency: String,
    pub amount: String,
    /// Of the month's expenses in that currency.
    pub percent: f64,
    pub count: usize,
    pub budget: Option<BudgetRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub month: String,
    pub label: String,
    pub income: String,
    pub expenses: String,
    pub saved: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardDebt {
    pub id: String,
    pub name: String,
    pub remaining: Vec<Money>,
    pub installments_left: u32,
    pub last_month: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonthDue {
    pub month: String,
    pub label: String,
    pub amounts: Vec<Money>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoanProgress {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub amount: String,
    pub paid_count: u32,
    pub count: u32,
    pub total_paid: String,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Debts {
    pub cards: Vec<CardDebt>,
    pub card_total: Vec<Money>,
    /// Card installments of the next months.
    pub upcoming: Vec<MonthDue>,
    pub loans: Vec<LoanProgress>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavingsTotal {
    pub currency: String,
    pub balance: String,
    pub saved_this_month: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    #[serde(flatten)]
    pub nav: MonthNav,
    pub balance: Vec<Balance>,
    pub pace: Option<Pace>,
    pub commitments: Commitments,
    pub categories: Vec<CategoryRow>,
    pub history: Vec<HistoryPoint>,
    pub debts: Debts,
    pub savings: Vec<SavingsTotal>,
    pub top_expenses: Vec<MovementRow>,
    pub recent: Vec<MovementRow>,
    /// Nothing loaded yet, ever.
    pub empty: bool,
}

pub fn finance_overview(app: crate::host::AppHandle, payload: MonthPayload) -> FinanceCommandResult<Overview> {
    check_month(&payload.month)?;
    read(&app, &payload.context, |connection| overview(connection, &payload.month))
}

fn percent(part: i128, whole: i128) -> Option<f64> {
    (whole > 0).then(|| ((part as f64 * 1000.0 / whole as f64).round()) / 10.0)
}

pub(crate) fn overview(connection: &Connection, month: &str) -> FinanceCommandResult<Overview> {
    let names = names(connection)?;
    let movements = month_movements(connection, month)?;
    let previous = month_movements(connection, &add_months(month, -1))?;
    let (from, to) = month_range(month).unwrap_or_default();
    let saved = model::saved_between(connection, &from, &to)?;
    let expenses = movements.iter().filter(|movement| movement.kind == "expense").collect::<Vec<_>>();

    // Balance, per currency.
    let mut currencies = movements.iter().map(|movement| movement.currency.clone()).collect::<Vec<_>>();
    currencies.extend(saved.keys().cloned());
    currencies.sort_by_key(|currency| (currency != "ARS", currency.clone()));
    currencies.dedup();
    let balance = currencies
        .iter()
        .map(|currency| {
            let of = |movement: &&Movement| &movement.currency == currency;
            let salary: i128 = movements.iter().filter(of).filter(|m| m.kind == "income" && m.category_id.as_deref() == Some(CATEGORY_SALARY.0)).map(|m| cents(&m.amount)).sum();
            let income: i128 = movements.iter().filter(of).filter(|m| m.kind == "income").map(|m| cents(&m.amount)).sum();
            let spent: i128 = movements.iter().filter(of).filter(|m| m.kind == "expense").map(|m| cents(&m.amount)).sum();
            let before: i128 = previous.iter().filter(of).filter(|m| m.kind == "expense").map(|m| cents(&m.amount)).sum();
            let saved_here = saved.get(currency).copied().unwrap_or(0);
            Balance {
                currency: currency.clone(),
                income: format_cents(income),
                salary: format_cents(salary),
                other_income: format_cents(income - salary),
                expenses: format_cents(spent),
                saved: format_cents(saved_here),
                left: format_cents(income - spent - saved_here),
                spent_percent: percent(spent, income),
                saved_percent: percent(saved_here.max(0), income),
                previous_expenses: format_cents(before),
                expenses_change: (before > 0).then(|| (((spent - before) as f64 * 1000.0 / before as f64).round()) / 10.0),
            }
        })
        .collect::<Vec<_>>();

    // Pace, in pesos.
    let pesos = expenses.iter().filter(|movement| movement.currency == "ARS").copied().collect::<Vec<_>>();
    let pace = (!pesos.is_empty()).then(|| {
        let total: i128 = pesos.iter().map(|movement| cents(&movement.amount)).sum();
        let days = days_in_month(month);
        let current = month == current_month();
        let elapsed = if current { today().get(8..10).and_then(|day| day.parse::<u32>().ok()).unwrap_or(days).max(1) } else { days };
        Pace {
            days_elapsed: elapsed,
            days_in_month: days,
            daily_average: format_cents(total / i128::from(elapsed)),
            projected: current.then(|| format_cents(total * i128::from(days) / i128::from(elapsed))),
            expense_count: pesos.len(),
            biggest: pesos.iter().max_by_key(|movement| cents(&movement.amount)).map(|movement| row(movement, &names)),
        }
    });

    // Categories, with their budgets.
    let budget_rows = budgets::budgets_of_month(connection, month)?;
    let mut by_category: BTreeMap<(String, Option<String>), (i128, usize)> = BTreeMap::new();
    for movement in &expenses {
        let entry = by_category.entry((movement.currency.clone(), movement.category_id.clone())).or_default();
        entry.0 += cents(&movement.amount);
        entry.1 += 1;
    }
    let spent_by_currency = totals_of(expenses.iter().copied());
    let mut categories = by_category
        .into_iter()
        .map(|((currency, category_id), (amount, count))| CategoryRow {
            name: category_id.as_ref().and_then(|id| names.categories.get(id)).cloned().unwrap_or_else(|| "Sin categoría".into()),
            budget: budget_rows.iter().find(|budget| Some(&budget.category_id) == category_id.as_ref() && budget.currency == currency).cloned(),
            percent: percent(amount, spent_by_currency.get(&currency).copied().unwrap_or(0)).unwrap_or(0.0),
            amount: format_cents(amount),
            category_id,
            currency,
            count,
        })
        .collect::<Vec<_>>();
    // Budgets of categories with nothing spent yet also show.
    for budget in &budget_rows {
        if !categories.iter().any(|row| row.category_id.as_deref() == Some(budget.category_id.as_str()) && row.currency == budget.currency) {
            categories.push(CategoryRow {
                category_id: Some(budget.category_id.clone()),
                name: budget.category_name.clone(),
                currency: budget.currency.clone(),
                amount: "0.00".into(),
                percent: 0.0,
                count: 0,
                budget: Some(budget.clone()),
            });
        }
    }
    categories.sort_by(|a, b| (a.currency != "ARS", -cents(&a.amount), &a.name).cmp(&(b.currency != "ARS", -cents(&b.amount), &b.name)));

    // Six months, in pesos.
    let mut history = Vec::new();
    for offset in (0..6).rev() {
        let point = add_months(month, -offset);
        let rows = month_movements(connection, &point)?;
        let (start, end) = month_range(&point).unwrap_or_default();
        let saved = model::saved_between(connection, &start, &end)?;
        let sum = |kind: &str| rows.iter().filter(|m| m.kind == kind && m.currency == "ARS").map(|m| cents(&m.amount)).sum::<i128>();
        history.push(HistoryPoint {
            label: month_short(&point),
            month: point,
            income: format_cents(sum("income")),
            expenses: format_cents(sum("expense")),
            saved: format_cents(saved.get("ARS").copied().unwrap_or(0)),
        });
    }

    let purchases = model::purchase_states(connection)?;
    let loans = model::loan_states(connection)?;
    let commitments = commitments(connection, month, &purchases, &loans)?;
    let debts = debts(connection, month, &purchases, &loans)?;

    // Savings.
    let balances = model::reserve_balances(connection)?;
    let mut savings_by_currency: BTreeMap<String, i128> = BTreeMap::new();
    for reserve in model::list_reserves(connection)? {
        *savings_by_currency.entry(reserve.currency.clone()).or_default() += balances.get(&reserve.id).copied().unwrap_or(0);
    }
    let mut savings = Vec::new();
    for (currency, balance) in &savings_by_currency {
        let month = saved_in_reserves(connection, &from, &to, currency)?;
        savings.push(SavingsTotal { currency: currency.clone(), balance: format_cents(*balance), saved_this_month: format_cents(month) });
    }
    savings.sort_by_key(|total| total.currency != "ARS");

    let mut top = expenses.clone();
    top.sort_by_key(|movement| -cents(&movement.amount));
    let empty: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM finance_movements) AND NOT EXISTS(SELECT 1 FROM finance_card_purchases) AND NOT EXISTS(SELECT 1 FROM finance_services) AND NOT EXISTS(SELECT 1 FROM finance_loans)",
        [],
        |row| row.get(0),
    )?;
    Ok(Overview {
        nav: nav(month),
        balance,
        pace,
        commitments,
        categories,
        history,
        debts,
        savings,
        top_expenses: top.iter().take(5).map(|movement| row(movement, &names)).collect(),
        recent: movements.iter().take(8).map(|movement| row(movement, &names)).collect(),
        empty,
    })
}

/// What reserves of `currency` grew in the dates, net.
fn saved_in_reserves(connection: &Connection, from: &str, to: &str, currency: &str) -> FinanceCommandResult<i128> {
    let reserves = model::list_reserves(connection)?.into_iter().filter(|reserve| reserve.currency == currency).map(|reserve| reserve.id).collect::<Vec<_>>();
    Ok(model::list_savings_movements(connection)?
        .into_iter()
        .filter(|movement| reserves.contains(&movement.reserve_id) && movement.date.as_str() >= from && movement.date.as_str() <= to)
        .map(|movement| if movement.kind == "deposit" { cents(&movement.amount) } else { -cents(&movement.amount) })
        .sum())
}

fn commitments(connection: &Connection, month: &str, purchases: &[PurchaseState], loans: &[model::LoanState]) -> FinanceCommandResult<Commitments> {
    let mut items = Vec::new();
    for service in model::services_of_month(connection, month)? {
        // A service costs what its payment says: before paying it, nothing.
        let (amount, status, subtitle) = match &service.paid {
            Some(paid) => (Some(paid.amount.clone()), "paid", format!("Pagado el {}", day_label(&paid.date))),
            None => (None, "pending", "Sin pagar".to_string()),
        };
        items.push(Commitment {
            kind: "service",
            id: service.service.id.clone(),
            title: service.service.name.clone(),
            subtitle,
            amount,
            currency: service.currency.clone().unwrap_or_else(|| "ARS".into()),
            status,
        });
    }
    for card in model::list_cards(connection)? {
        let due = model::card_installments(purchases, &card.id, month);
        let mut by_currency: BTreeMap<String, (i128, i128, usize, bool)> = BTreeMap::new();
        for installment in &due {
            let entry = by_currency.entry(installment.currency.clone()).or_default();
            if installment.paid.is_some() {
                entry.0 += cents(&installment.amount);
            } else {
                entry.1 += cents(&installment.amount);
                entry.3 |= installment.late;
            }
            entry.2 += 1;
        }
        for (currency, (paid, pending, count, late)) in by_currency {
            let status = if pending == 0 { "paid" } else if late { "late" } else { "pending" };
            let subtitle = format!("{count} {}", if count == 1 { "cuota" } else { "cuotas" });
            items.push(Commitment {
                kind: "card",
                id: card.id.clone(),
                title: card.name.clone(),
                subtitle,
                amount: Some(format_cents(if pending == 0 { paid } else { pending })),
                currency,
                status,
            });
        }
    }
    for state in loans {
        let Some(installment) = state.installment_of(month) else { continue };
        let late = state.unpaid_before(month) > 0;
        // Each installment costs something else: before paying it, no amount.
        let (amount, status) = match &installment.paid {
            Some(paid) => (Some(paid.amount.clone()), "paid"),
            None => (None, if late { "late" } else { "pending" }),
        };
        items.push(Commitment {
            kind: "loan",
            id: state.loan.id.clone(),
            title: state.loan.name.clone(),
            subtitle: format!("Cuota {} de {}", installment.number, state.loan.installment_count),
            amount,
            currency: state.loan.currency.clone(),
            status,
        });
    }
    let mut paid = BTreeMap::new();
    let mut pending = BTreeMap::new();
    for item in &items {
        let target = if item.status == "paid" { &mut paid } else { &mut pending };
        *target.entry(item.currency.clone()).or_insert(0) += item.amount.as_deref().map(cents).unwrap_or(0);
    }
    items.sort_by_key(|item| (item.status == "paid", item.status != "late"));
    let unknown = |kind: &str| items.iter().filter(|item| item.kind == kind && item.status != "paid" && item.amount.is_none()).count();
    Ok(Commitments {
        paid_count: items.iter().filter(|item| item.status == "paid").count(),
        pending_count: items.iter().filter(|item| item.status != "paid").count(),
        pending_note: unknown_note(unknown("service"), unknown("loan")),
        paid: money_list(&paid),
        pending: money_list(&pending),
        items,
    })
}

/// What is still to pay without an amount: «2 servicios», «1 cuota de préstamo».
pub(crate) fn unknown_note(services: usize, loans: usize) -> Option<String> {
    let parts = [
        (services > 0).then(|| if services == 1 { "1 servicio".to_string() } else { format!("{services} servicios") }),
        (loans > 0).then(|| if loans == 1 { "1 cuota de préstamo".to_string() } else { format!("{loans} cuotas de préstamo") }),
    ];
    let parts = parts.into_iter().flatten().collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" y "))
}

fn debts(connection: &Connection, month: &str, purchases: &[PurchaseState], loans: &[model::LoanState]) -> FinanceCommandResult<Debts> {
    let mut cards = Vec::new();
    let mut card_total = BTreeMap::new();
    for card in model::list_cards(connection)? {
        let mut remaining = BTreeMap::new();
        let mut left = 0;
        let mut last: Option<String> = None;
        for state in purchases.iter().filter(|state| state.purchase.card_id == card.id && !state.finished) {
            *remaining.entry(state.purchase.currency.clone()).or_insert(0) += cents(&state.remaining);
            *card_total.entry(state.purchase.currency.clone()).or_insert(0) += cents(&state.remaining);
            left += state.purchase.installment_count - state.paid_count;
            let end = state.last_month();
            if last.as_ref().is_none_or(|last| &end > last) {
                last = Some(end);
            }
        }
        if left > 0 {
            cards.push(CardDebt { id: card.id, name: card.name, remaining: money_list(&remaining), installments_left: left, last_month: last.map(|month| month_label(&month)) });
        }
    }
    let upcoming = (1..=3)
        .map(|offset| {
            let point = add_months(month, offset);
            let mut amounts = BTreeMap::new();
            for state in purchases {
                for installment in state.installments.iter().filter(|installment| installment.month == point && installment.paid.is_none()) {
                    *amounts.entry(state.purchase.currency.clone()).or_insert(0) += cents(&installment.amount);
                }
            }
            MonthDue { label: month_label(&point), month: point, amounts: money_list(&amounts) }
        })
        .collect();
    let loans = loans
        .iter()
        .filter(|state| !state.finished)
        .map(|state| LoanProgress {
            id: state.loan.id.clone(),
            name: state.loan.name.clone(),
            currency: state.loan.currency.clone(),
            amount: state.loan.amount.clone(),
            paid_count: state.paid_count,
            count: state.loan.installment_count,
            total_paid: state.total_paid.clone(),
            percent: percent(i128::from(state.paid_count), i128::from(state.loan.installment_count)).unwrap_or(0.0),
        })
        .collect();
    Ok(Debts { cards, card_total: money_list(&card_total), upcoming, loans })
}

// ---------------------------------------------------------------------------
// Payments of the month
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServicePayment {
    pub id: String,
    pub name: String,
    /// The month's payment: its amount is the only one shown.
    pub paid: Option<model::Payment>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstallmentRow {
    pub purchase_id: String,
    pub name: String,
    pub number: u32,
    pub count: u32,
    pub month: String,
    pub amount: String,
    pub currency: String,
    pub paid: Option<model::Payment>,
    pub late: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseRow {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub total: String,
    pub currency: String,
    pub count: u32,
    pub paid_count: u32,
    pub remaining: String,
    pub first_month: String,
    pub last_month: String,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardMonth {
    pub id: String,
    pub name: String,
    /// The installments of the month (and earlier ones unpaid).
    pub installments: Vec<InstallmentRow>,
    pub total: Vec<Money>,
    pub pending: Vec<Money>,
    pub pending_count: usize,
    /// Everything the card still owes.
    pub debt: Vec<Money>,
    pub purchases: Vec<PurchaseRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoanMonth {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub amount: String,
    pub count: u32,
    /// The installment that falls in the month.
    pub number: Option<u32>,
    pub paid: Option<model::Payment>,
    pub next_number: Option<u32>,
    pub paid_count: u32,
    pub total_paid: String,
    pub late_count: usize,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Payments {
    #[serde(flatten)]
    pub nav: MonthNav,
    pub services: Vec<ServicePayment>,
    pub cards: Vec<CardMonth>,
    pub loans: Vec<LoanMonth>,
    pub paid: Vec<Money>,
    /// What is still to pay and is known exactly (card installments).
    pub pending: Vec<Money>,
    /// What is still to pay without an amount yet.
    pub pending_note: Option<String>,
}

pub fn finance_payments(app: crate::host::AppHandle, payload: MonthPayload) -> FinanceCommandResult<Payments> {
    check_month(&payload.month)?;
    read(&app, &payload.context, |connection| payments(connection, &payload.month))
}

pub(crate) fn payments(connection: &Connection, month: &str) -> FinanceCommandResult<Payments> {
    let categories = model::list_categories(connection)?.into_iter().map(|category: Category| (category.id, category.name)).collect::<HashMap<_, _>>();
    let purchases = model::purchase_states(connection)?;
    let loans = model::loan_states(connection)?;
    let mut paid = BTreeMap::new();
    let mut pending = BTreeMap::new();
    let services = model::services_of_month(connection, month)?
        .into_iter()
        .map(|service| {
            if let (Some(payment), Some(currency)) = (&service.paid, &service.currency) {
                *paid.entry(currency.clone()).or_insert(0) += cents(&payment.amount);
            }
            ServicePayment { id: service.service.id, name: service.service.name, paid: service.paid, currency: service.currency }
        })
        .collect::<Vec<_>>();
    let cards = model::list_cards(connection)?
        .into_iter()
        .map(|card| {
            let installments = model::card_installments(&purchases, &card.id, month)
                .into_iter()
                .map(|installment| InstallmentRow {
                    purchase_id: installment.purchase_id,
                    name: installment.name,
                    number: installment.number,
                    count: installment.count,
                    month: installment.month,
                    amount: installment.amount,
                    currency: installment.currency,
                    paid: installment.paid,
                    late: installment.late,
                })
                .collect::<Vec<_>>();
            let mut total = BTreeMap::new();
            let mut card_pending = BTreeMap::new();
            for installment in &installments {
                *total.entry(installment.currency.clone()).or_insert(0) += cents(&installment.amount);
                let target = if installment.paid.is_some() { &mut paid } else { &mut pending };
                *target.entry(installment.currency.clone()).or_insert(0) += cents(&installment.amount);
                if installment.paid.is_none() {
                    *card_pending.entry(installment.currency.clone()).or_insert(0) += cents(&installment.amount);
                }
            }
            let mut debt = BTreeMap::new();
            let mut rows = Vec::new();
            for state in purchases.iter().filter(|state| state.purchase.card_id == card.id) {
                if !state.finished {
                    *debt.entry(state.purchase.currency.clone()).or_insert(0) += cents(&state.remaining);
                }
                // Finished purchases show for a while after their last month.
                if !state.finished || crate::finance::months_between(&state.last_month(), month) <= 1 {
                    rows.push(PurchaseRow {
                        id: state.purchase.id.clone(),
                        name: state.purchase.name.clone(),
                        category: state.purchase.category_id.as_ref().and_then(|id| categories.get(id)).cloned(),
                        total: state.purchase.total.clone(),
                        currency: state.purchase.currency.clone(),
                        count: state.purchase.installment_count,
                        paid_count: state.paid_count,
                        remaining: state.remaining.clone(),
                        first_month: state.purchase.first_month.clone(),
                        last_month: state.last_month(),
                        finished: state.finished,
                    });
                }
            }
            rows.sort_by_key(|row| (row.finished, std::cmp::Reverse(row.first_month.clone())));
            CardMonth {
                pending_count: installments.iter().filter(|installment| installment.paid.is_none()).count(),
                id: card.id,
                name: card.name,
                installments,
                total: money_list(&total),
                pending: money_list(&card_pending),
                debt: money_list(&debt),
                purchases: rows,
            }
        })
        .collect();
    let loans: Vec<LoanMonth> = loans
        .iter()
        .filter(|state| !state.finished || state.installments.iter().any(|installment| installment.paid.as_ref().is_some_and(|paid| paid.date.starts_with(month))))
        .map(|state| {
            let installment = state.installment_of(month);
            if let Some(payment) = installment.and_then(|installment| installment.paid.as_ref()) {
                *paid.entry(state.loan.currency.clone()).or_insert(0) += cents(&payment.amount);
            }
            LoanMonth {
                id: state.loan.id.clone(),
                name: state.loan.name.clone(),
                currency: state.loan.currency.clone(),
                amount: state.loan.amount.clone(),
                count: state.loan.installment_count,
                number: installment.map(|installment| installment.number),
                paid: installment.and_then(|installment| installment.paid.clone()),
                next_number: state.next_number,
                paid_count: state.paid_count,
                total_paid: state.total_paid.clone(),
                late_count: state.unpaid_before(month),
                finished: state.finished,
            }
        })
        .collect();
    let services_left = services.iter().filter(|service| service.paid.is_none()).count();
    let loans_left = loans.iter().filter(|loan| loan.number.is_some() && loan.paid.is_none() && !loan.finished).count();
    Ok(Payments {
        nav: nav(month),
        pending_note: unknown_note(services_left, loans_left),
        services,
        cards,
        loans,
        paid: money_list(&paid),
        pending: money_list(&pending),
    })
}

// ---------------------------------------------------------------------------
// Movements and history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementFilter {
    /// `expense`, `income` or empty for both.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub category_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub search: String,
}

impl MovementFilter {
    fn keeps(&self, movement: &MovementRow) -> bool {
        let folded = crate::finance_ops::fold(&self.search);
        (self.kind.is_empty() || movement.kind == self.kind)
            && (self.category_id.is_empty() || movement.category_id.as_deref() == Some(self.category_id.as_str()) || (self.category_id == "none" && movement.category_id.is_none()))
            && (self.account_id.is_empty() || self.account_id_matches(movement))
            && (folded.is_empty()
                || [Some(&movement.name), movement.category.as_ref(), movement.detail.as_ref(), movement.account.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|text| crate::finance_ops::fold(text).contains(&folded))
                || crate::finance::typed_cents(&serde_json::Value::String(self.search.clone())).is_some_and(|wanted| wanted == cents(&movement.amount)))
    }

    fn account_id_matches(&self, movement: &MovementRow) -> bool {
        // Rows carry the account's name; the id is matched by the caller.
        movement.account.as_deref() == Some(self.account_id.as_str())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementsPayload {
    pub context: FinanceContext,
    pub month: String,
    #[serde(default)]
    pub filter: MovementFilter,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilterOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovementsView {
    #[serde(flatten)]
    pub nav: MonthNav,
    pub rows: Vec<MovementRow>,
    pub income: Vec<Money>,
    pub expenses: Vec<Money>,
    pub categories: Vec<FilterOption>,
    pub accounts: Vec<FilterOption>,
}

/// The filter with the account's name in place of its id.
fn resolved(filter: &MovementFilter, names: &Names) -> MovementFilter {
    let mut filter = filter.clone();
    if let Some(name) = names.accounts.get(&filter.account_id) {
        filter.account_id = name.clone();
    }
    filter
}

pub fn finance_movements(app: crate::host::AppHandle, payload: MovementsPayload) -> FinanceCommandResult<MovementsView> {
    check_month(&payload.month)?;
    read(&app, &payload.context, |connection| movements_view(connection, &payload.month, &payload.filter))
}

pub(crate) fn movements_view(connection: &Connection, month: &str, filter: &MovementFilter) -> FinanceCommandResult<MovementsView> {
    let names = names(connection)?;
    let filter = resolved(filter, &names);
    let rows = month_movements(connection, month)?.iter().map(|movement| row(movement, &names)).filter(|row| filter.keeps(row)).collect::<Vec<_>>();
    let sum = |kind: &str| {
        let mut totals = BTreeMap::new();
        for row in rows.iter().filter(|row| row.kind == kind) {
            *totals.entry(row.currency.clone()).or_insert(0) += cents(&row.amount);
        }
        money_list(&totals)
    };
    Ok(MovementsView {
        nav: nav(month),
        income: sum("income"),
        expenses: sum("expense"),
        categories: model::list_categories(connection)?.into_iter().map(|category| FilterOption { id: category.id, name: category.name }).collect(),
        accounts: model::list_accounts(connection)?.into_iter().map(|account| FilterOption { id: account.id, name: account.name }).collect(),
        rows,
    })
}

/// Every movement between two dates, oldest first, as rows (for exports).
pub(crate) fn rows_between(connection: &Connection, from: &str, to: &str) -> FinanceCommandResult<Vec<MovementRow>> {
    let names = names(connection)?;
    let mut rows = model::movements_between(connection, from, to)?.iter().map(|movement| row(movement, &names)).collect::<Vec<_>>();
    rows.reverse();
    Ok(rows)
}

const SEARCH_PAGE: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPayload {
    pub context: FinanceContext,
    #[serde(default)]
    pub filter: MovementFilter,
    #[serde(default)]
    pub page: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchMonth {
    pub month: String,
    pub label: String,
    pub rows: Vec<MovementRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub count: usize,
    pub page: usize,
    pub has_more: bool,
    pub months: Vec<SearchMonth>,
    pub income: Vec<Money>,
    pub expenses: Vec<Money>,
}

pub fn finance_search(app: crate::host::AppHandle, payload: SearchPayload) -> FinanceCommandResult<SearchResult> {
    read(&app, &payload.context, |connection| search(connection, &payload.filter, payload.page))
}

pub(crate) fn search(connection: &Connection, filter: &MovementFilter, page: usize) -> FinanceCommandResult<SearchResult> {
    let names = names(connection)?;
    let filter = resolved(filter, &names);
    let all = model::movements_between(connection, "0000-01-01", "9999-12-31")?
        .iter()
        .map(|movement| row(movement, &names))
        .filter(|row| filter.keeps(row))
        .collect::<Vec<_>>();
    let mut income = BTreeMap::new();
    let mut expenses = BTreeMap::new();
    for row in &all {
        let target = if row.kind == "income" { &mut income } else { &mut expenses };
        *target.entry(row.currency.clone()).or_insert(0) += cents(&row.amount);
    }
    let start = page * SEARCH_PAGE;
    let mut months: Vec<SearchMonth> = Vec::new();
    for row in all.iter().skip(start).take(SEARCH_PAGE) {
        let month = row.date.get(0..7).unwrap_or_default().to_string();
        match months.last_mut() {
            Some(last) if last.month == month => last.rows.push(row.clone()),
            _ => months.push(SearchMonth { label: month_label(&month), month, rows: vec![row.clone()] }),
        }
    }
    Ok(SearchResult { count: all.len(), page, has_more: all.len() > start + SEARCH_PAGE, months, income: money_list(&income), expenses: money_list(&expenses) })
}

// ---------------------------------------------------------------------------
// Savings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReserveRow {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub balance: String,
    pub goal: Option<String>,
    pub goal_percent: Option<f64>,
    pub saved_this_month: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavingsRow {
    pub id: String,
    /// `savings` or `exchange`: the form that edits it.
    pub kind: &'static str,
    pub title: String,
    pub reserve: String,
    /// `deposit` or `withdrawal`.
    pub direction: String,
    pub amount: String,
    pub currency: String,
    pub pesos: Option<String>,
    pub date: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DollarValue {
    pub quote: String,
    pub rate: f64,
    pub pesos: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsView {
    pub reserves: Vec<ReserveRow>,
    pub totals: Vec<SavingsTotal>,
    /// The dollars saved, in pesos at the blue (or official) buying rate.
    pub dollars_in_pesos: Option<DollarValue>,
    pub movements: Vec<SavingsRow>,
    pub quotes: Vec<crate::services::finance_external::DollarQuote>,
    pub quotes_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsPayload {
    pub context: FinanceContext,
}

pub async fn finance_savings(app: crate::host::AppHandle, payload: SavingsPayload) -> FinanceCommandResult<SavingsView> {
    let mut view = read(&app, &payload.context, savings)?;
    match crate::services::finance_external::dollar_quotes().await {
        Ok(quotes) => {
            let dollars = view.totals.iter().find(|total| total.currency == "USD").map(|total| cents(&total.balance)).unwrap_or(0);
            let quote = ["blue", "oficial"].iter().find_map(|kind| quotes.iter().find(|quote| quote.kind == *kind));
            if let (Some(quote), true) = (quote, dollars > 0) {
                let pesos = (dollars as f64 * quote.buy).round() as i128;
                view.dollars_in_pesos = Some(DollarValue { quote: quote.name.clone(), rate: quote.buy, pesos: format_cents(pesos) });
            }
            view.quotes = quotes;
        }
        Err(error) => view.quotes_error = Some(error),
    }
    Ok(view)
}

pub(crate) fn savings(connection: &Connection) -> FinanceCommandResult<SavingsView> {
    let reserves = model::list_reserves(connection)?;
    let balances = model::reserve_balances(connection)?;
    let (from, to) = month_range(&current_month()).unwrap_or_default();
    let movements = model::list_savings_movements(connection)?;
    let rows = reserves
        .iter()
        .map(|reserve| {
            let balance = balances.get(&reserve.id).copied().unwrap_or(0);
            let month: i128 = movements
                .iter()
                .filter(|movement| movement.reserve_id == reserve.id && movement.date.as_str() >= from.as_str() && movement.date.as_str() <= to.as_str())
                .map(|movement| if movement.kind == "deposit" { cents(&movement.amount) } else { -cents(&movement.amount) })
                .sum();
            ReserveRow {
                id: reserve.id.clone(),
                name: reserve.name.clone(),
                currency: reserve.currency.clone(),
                balance: format_cents(balance),
                goal_percent: reserve.goal.as_deref().and_then(|goal| percent(balance.max(0), cents(goal))).map(|value| value.min(100.0)),
                goal: reserve.goal.clone(),
                saved_this_month: format_cents(month),
            }
        })
        .collect::<Vec<_>>();
    let mut totals: BTreeMap<String, (i128, i128)> = BTreeMap::new();
    for reserve in &rows {
        let entry = totals.entry(reserve.currency.clone()).or_default();
        entry.0 += cents(&reserve.balance);
        entry.1 += cents(&reserve.saved_this_month);
    }
    let mut totals = totals
        .into_iter()
        .map(|(currency, (balance, month))| SavingsTotal { currency, balance: format_cents(balance), saved_this_month: format_cents(month) })
        .collect::<Vec<_>>();
    totals.sort_by_key(|total| total.currency != "ARS");
    let reserve_names = reserves.iter().map(|reserve| (reserve.id.clone(), (reserve.name.clone(), reserve.currency.clone()))).collect::<HashMap<_, _>>();
    let movements = movements
        .into_iter()
        .take(30)
        .map(|movement| {
            let (reserve, currency) = reserve_names.get(&movement.reserve_id).cloned().unwrap_or_default();
            let exchange = movement.pesos.is_some();
            let title = match (exchange, movement.kind.as_str()) {
                (true, "deposit") => "Compra de dólares".to_string(),
                (true, _) => "Venta de dólares".to_string(),
                (false, "deposit") => "Guardaste".to_string(),
                (false, _) => "Sacaste".to_string(),
            };
            SavingsRow {
                id: movement.id,
                kind: if exchange { "exchange" } else { "savings" },
                title,
                reserve,
                direction: movement.kind,
                amount: movement.amount,
                currency,
                pesos: movement.pesos,
                date: movement.date,
                note: movement.note,
            }
        })
        .collect();
    Ok(SavingsView { reserves: rows, totals, dollars_in_pesos: None, movements, quotes: Vec::new(), quotes_error: None })
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceHomeSummary {
    expenses: Vec<Money>,
    expense_count: usize,
    income: Vec<Money>,
    /// What is left of the month: income minus expenses minus savings.
    left: Vec<Money>,
    /// What is still to pay this month and is known exactly (card installments).
    pending: Vec<Money>,
    /// What is still to pay without an amount yet: «2 servicios».
    pending_note: Option<String>,
    pending_count: usize,
    saved: Vec<Money>,
}

pub(crate) fn home_summary(app: &crate::host::AppHandle, context: &FinanceContext, month: &str) -> FinanceCommandResult<FinanceHomeSummary> {
    check_month(month)?;
    read(app, context, |connection| {
        let overview = overview(connection, month)?;
        let pick = |field: fn(&Balance) -> &String| {
            let mut totals = BTreeMap::new();
            for balance in &overview.balance {
                totals.insert(balance.currency.clone(), cents(field(balance)));
            }
            money_list(&totals)
        };
        Ok(FinanceHomeSummary {
            expenses: pick(|balance| &balance.expenses),
            expense_count: overview.pace.as_ref().map(|pace| pace.expense_count).unwrap_or(0),
            income: pick(|balance| &balance.income),
            left: pick(|balance| &balance.left),
            pending: overview.commitments.pending.clone(),
            pending_note: overview.commitments.pending_note.clone(),
            pending_count: overview.commitments.pending_count,
            saved: pick(|balance| &balance.saved),
        })
    })
}

