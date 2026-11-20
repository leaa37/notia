//! Lectura de Finanzas: los registros guardados y lo que se calcula de
//! ellos (las cuotas de cada compra con tarjeta, el estado de cada préstamo
//! y de cada servicio en un mes, el saldo de cada reserva).
//!
//! Nada de esto se guarda: una cuota existe porque la compra tiene cuotas,
//! y está pagada si hay un gasto que la paga.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Params, Row};
use serde::Serialize;

use crate::finance::{add_months, installment_amounts, months_between, parse_cents};

pub(crate) type ReadResult<T> = rusqlite::Result<T>;

fn rows<T, P: Params>(connection: &Connection, sql: &str, params: P, map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>) -> ReadResult<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params, map)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub(crate) fn cents(value: &str) -> i128 {
    parse_cents(value).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    /// `expense` or `income`.
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Named {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Loan {
    pub id: String,
    pub name: String,
    pub currency: String,
    /// What was lent.
    pub amount: String,
    pub installment_count: u32,
    /// The month of the first installment (`YYYY-MM`).
    pub first_month: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardPurchase {
    pub id: String,
    pub card_id: String,
    pub name: String,
    pub category_id: Option<String>,
    pub currency: String,
    /// What the purchase costs in all, interest included.
    pub total: String,
    pub installment_count: u32,
    pub first_month: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Movement {
    pub id: String,
    /// `expense` or `income`.
    pub kind: String,
    pub name: String,
    pub category_id: Option<String>,
    pub account_id: Option<String>,
    pub amount: String,
    pub currency: String,
    pub date: String,
    pub service_id: Option<String>,
    pub service_month: Option<String>,
    pub card_purchase_id: Option<String>,
    pub loan_id: Option<String>,
    pub installment_number: Option<u32>,
    pub source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reserve {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub goal: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavingsMovement {
    pub id: String,
    pub reserve_id: String,
    /// `deposit` or `withdrawal`.
    pub kind: String,
    /// In the reserve's currency.
    pub amount: String,
    /// Pesos paid for dollars bought, or received for dollars sold.
    pub pesos: Option<String>,
    pub date: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub id: String,
    pub category_id: String,
    pub currency: String,
    pub amount: String,
}

pub(crate) fn list_categories(connection: &Connection) -> ReadResult<Vec<Category>> {
    rows(connection, "SELECT id,name,kind FROM finance_categories ORDER BY kind DESC, name COLLATE NOCASE", [], |row| {
        Ok(Category { id: row.get(0)?, name: row.get(1)?, kind: row.get(2)? })
    })
}

fn list_named(connection: &Connection, table: &str) -> ReadResult<Vec<Named>> {
    rows(connection, &format!("SELECT id,name FROM {table} ORDER BY name COLLATE NOCASE"), [], |row| Ok(Named { id: row.get(0)?, name: row.get(1)? }))
}

pub(crate) fn list_accounts(connection: &Connection) -> ReadResult<Vec<Named>> {
    list_named(connection, "finance_accounts")
}

pub(crate) fn list_cards(connection: &Connection) -> ReadResult<Vec<Named>> {
    list_named(connection, "finance_cards")
}

pub(crate) fn list_services(connection: &Connection) -> ReadResult<Vec<Service>> {
    rows(connection, "SELECT id,name,currency,amount FROM finance_services ORDER BY name COLLATE NOCASE", [], |row| {
        Ok(Service { id: row.get(0)?, name: row.get(1)?, currency: row.get(2)?, amount: row.get(3)? })
    })
}

fn loan_from(row: &Row<'_>) -> rusqlite::Result<Loan> {
    Ok(Loan {
        id: row.get(0)?,
        name: row.get(1)?,
        currency: row.get(2)?,
        amount: row.get(3)?,
        installment_count: row.get(4)?,
        first_month: row.get(5)?,
    })
}

const LOAN_COLUMNS: &str = "id,name,currency,amount,installment_count,first_month";

pub(crate) fn list_loans(connection: &Connection) -> ReadResult<Vec<Loan>> {
    rows(connection, &format!("SELECT {LOAN_COLUMNS} FROM finance_loans ORDER BY name COLLATE NOCASE"), [], loan_from)
}

pub(crate) fn load_loan(connection: &Connection, id: &str) -> ReadResult<Option<Loan>> {
    connection.query_row(&format!("SELECT {LOAN_COLUMNS} FROM finance_loans WHERE id=?1"), [id], loan_from).optional()
}

fn purchase_from(row: &Row<'_>) -> rusqlite::Result<CardPurchase> {
    Ok(CardPurchase {
        id: row.get(0)?,
        card_id: row.get(1)?,
        name: row.get(2)?,
        category_id: row.get(3)?,
        currency: row.get(4)?,
        total: row.get(5)?,
        installment_count: row.get(6)?,
        first_month: row.get(7)?,
        created_at: row.get(8)?,
    })
}

const PURCHASE_COLUMNS: &str = "id,card_id,name,category_id,currency,total,installment_count,first_month,created_at";

pub(crate) fn list_purchases(connection: &Connection) -> ReadResult<Vec<CardPurchase>> {
    rows(
        connection,
        &format!("SELECT {PURCHASE_COLUMNS} FROM finance_card_purchases ORDER BY first_month DESC, created_at DESC"),
        [],
        purchase_from,
    )
}

const MOVEMENT_COLUMNS: &str = "id,kind,name,category_id,account_id,amount,currency,date,service_id,service_month,card_purchase_id,loan_id,installment_number,source,created_at";

fn movement_from(row: &Row<'_>) -> rusqlite::Result<Movement> {
    Ok(Movement {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        category_id: row.get(3)?,
        account_id: row.get(4)?,
        amount: row.get(5)?,
        currency: row.get(6)?,
        date: row.get(7)?,
        service_id: row.get(8)?,
        service_month: row.get(9)?,
        card_purchase_id: row.get(10)?,
        loan_id: row.get(11)?,
        installment_number: row.get(12)?,
        source: row.get(13)?,
        created_at: row.get(14)?,
    })
}

pub(crate) fn load_movement(connection: &Connection, id: &str) -> ReadResult<Option<Movement>> {
    connection.query_row(&format!("SELECT {MOVEMENT_COLUMNS} FROM finance_movements WHERE id=?1"), [id], movement_from).optional()
}

/// The movements between two dates, the most recent first.
pub(crate) fn movements_between(connection: &Connection, from: &str, to: &str) -> ReadResult<Vec<Movement>> {
    rows(
        connection,
        &format!("SELECT {MOVEMENT_COLUMNS} FROM finance_movements WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC, created_at DESC"),
        params![from, to],
        movement_from,
    )
}

/// Every payment of a card purchase, loan or service.
fn payments(connection: &Connection, column: &str) -> ReadResult<Vec<Movement>> {
    rows(
        connection,
        &format!("SELECT {MOVEMENT_COLUMNS} FROM finance_movements WHERE {column} IS NOT NULL ORDER BY date, created_at"),
        [],
        movement_from,
    )
}

pub(crate) fn list_reserves(connection: &Connection) -> ReadResult<Vec<Reserve>> {
    rows(connection, "SELECT id,name,currency,goal FROM finance_reserves ORDER BY currency, name COLLATE NOCASE", [], |row| {
        Ok(Reserve { id: row.get(0)?, name: row.get(1)?, currency: row.get(2)?, goal: row.get(3)? })
    })
}

const SAVINGS_COLUMNS: &str = "id,reserve_id,kind,amount,pesos,date,note";

fn savings_from(row: &Row<'_>) -> rusqlite::Result<SavingsMovement> {
    Ok(SavingsMovement {
        id: row.get(0)?,
        reserve_id: row.get(1)?,
        kind: row.get(2)?,
        amount: row.get(3)?,
        pesos: row.get(4)?,
        date: row.get(5)?,
        note: row.get(6)?,
    })
}

pub(crate) fn list_savings_movements(connection: &Connection) -> ReadResult<Vec<SavingsMovement>> {
    rows(connection, &format!("SELECT {SAVINGS_COLUMNS} FROM finance_savings_movements ORDER BY date DESC, created_at DESC"), [], savings_from)
}

pub(crate) fn load_savings_movement(connection: &Connection, id: &str) -> ReadResult<Option<SavingsMovement>> {
    connection.query_row(&format!("SELECT {SAVINGS_COLUMNS} FROM finance_savings_movements WHERE id=?1"), [id], savings_from).optional()
}

pub(crate) fn list_budgets(connection: &Connection) -> ReadResult<Vec<Budget>> {
    rows(
        connection,
        "SELECT b.id,b.category_id,b.currency,b.amount FROM finance_budgets b JOIN finance_categories c ON c.id=b.category_id ORDER BY c.name COLLATE NOCASE, b.currency",
        [],
        |row| Ok(Budget { id: row.get(0)?, category_id: row.get(1)?, currency: row.get(2)?, amount: row.get(3)? }),
    )
}

// ---------------------------------------------------------------------------
// Card purchases and their installments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub movement_id: String,
    pub date: String,
    pub amount: String,
}

impl Payment {
    fn of(movement: &Movement) -> Self {
        Payment { movement_id: movement.id.clone(), date: movement.date.clone(), amount: movement.amount.clone() }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Installment {
    pub number: u32,
    pub month: String,
    pub amount: String,
    pub paid: Option<Payment>,
}

/// A card purchase with its installments and what is paid.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseState {
    #[serde(flatten)]
    pub purchase: CardPurchase,
    pub installments: Vec<Installment>,
    pub paid_count: u32,
    /// What is left to pay, by the installments' amounts.
    pub remaining: String,
    pub finished: bool,
}

impl PurchaseState {
    pub(crate) fn installment(&self, number: u32) -> Option<&Installment> {
        self.installments.iter().find(|installment| installment.number == number)
    }

    pub(crate) fn last_month(&self) -> String {
        add_months(&self.purchase.first_month, self.purchase.installment_count as i32 - 1)
    }
}

/// Every purchase with its installments; a payment that is not there (the
/// purchase shrank) is ignored.
pub(crate) fn purchase_states(connection: &Connection) -> ReadResult<Vec<PurchaseState>> {
    let mut paid: HashMap<(String, u32), Movement> = HashMap::new();
    for movement in payments(connection, "card_purchase_id")? {
        if let (Some(purchase), Some(number)) = (movement.card_purchase_id.clone(), movement.installment_number) {
            paid.insert((purchase, number), movement);
        }
    }
    Ok(list_purchases(connection)?.into_iter().map(|purchase| purchase_state(purchase, &paid)).collect())
}

fn purchase_state(purchase: CardPurchase, paid: &HashMap<(String, u32), Movement>) -> PurchaseState {
    let amounts = installment_amounts(cents(&purchase.total), purchase.installment_count);
    let installments = amounts
        .iter()
        .enumerate()
        .map(|(index, amount)| {
            let number = index as u32 + 1;
            Installment {
                number,
                month: add_months(&purchase.first_month, index as i32),
                amount: crate::finance::format_cents(*amount),
                paid: paid.get(&(purchase.id.clone(), number)).map(Payment::of),
            }
        })
        .collect::<Vec<_>>();
    let paid_count = installments.iter().filter(|installment| installment.paid.is_some()).count() as u32;
    let remaining: i128 = installments.iter().filter(|installment| installment.paid.is_none()).map(|installment| cents(&installment.amount)).sum();
    PurchaseState {
        finished: paid_count == purchase.installment_count,
        purchase,
        installments,
        paid_count,
        remaining: crate::finance::format_cents(remaining),
    }
}

pub(crate) fn purchase_state_of(connection: &Connection, id: &str) -> ReadResult<Option<PurchaseState>> {
    Ok(purchase_states(connection)?.into_iter().find(|state| state.purchase.id == id))
}

/// One installment of a card, for the month's list.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardInstallment {
    pub purchase_id: String,
    pub name: String,
    pub category_id: Option<String>,
    pub currency: String,
    pub number: u32,
    pub count: u32,
    pub month: String,
    pub amount: String,
    pub paid: Option<Payment>,
    /// From an earlier month and still unpaid.
    pub late: bool,
}

/// What a card asks in `month`: its installments due that month, and the
/// earlier ones still unpaid.
pub(crate) fn card_installments(states: &[PurchaseState], card_id: &str, month: &str) -> Vec<CardInstallment> {
    let mut due = Vec::new();
    for state in states.iter().filter(|state| state.purchase.card_id == card_id) {
        for installment in &state.installments {
            let late = installment.month.as_str() < month && installment.paid.is_none();
            if installment.month == month || late {
                due.push(CardInstallment {
                    purchase_id: state.purchase.id.clone(),
                    name: state.purchase.name.clone(),
                    category_id: state.purchase.category_id.clone(),
                    currency: state.purchase.currency.clone(),
                    number: installment.number,
                    count: state.purchase.installment_count,
                    month: installment.month.clone(),
                    amount: installment.amount.clone(),
                    paid: installment.paid.clone(),
                    late,
                });
            }
        }
    }
    due.sort_by(|a, b| (!a.late, &a.month, &a.name, a.number).cmp(&(!b.late, &b.month, &b.name, b.number)));
    due
}

// ---------------------------------------------------------------------------
// Loans
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoanInstallment {
    pub number: u32,
    pub month: String,
    pub paid: Option<Payment>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoanState {
    #[serde(flatten)]
    pub loan: Loan,
    pub installments: Vec<LoanInstallment>,
    pub paid_count: u32,
    pub total_paid: String,
    /// What the last payment was: the amount the next one starts from.
    pub last_paid: Option<String>,
    /// The first installment not paid yet.
    pub next_number: Option<u32>,
    pub finished: bool,
}

impl LoanState {
    /// The installment that falls in `month`, if the loan runs then.
    pub(crate) fn installment_of(&self, month: &str) -> Option<&LoanInstallment> {
        let offset = months_between(&self.loan.first_month, month);
        (offset >= 0).then(|| self.installments.get(offset as usize)).flatten()
    }

    pub(crate) fn unpaid_before(&self, month: &str) -> usize {
        self.installments.iter().filter(|installment| installment.month.as_str() < month && installment.paid.is_none()).count()
    }
}

pub(crate) fn loan_states(connection: &Connection) -> ReadResult<Vec<LoanState>> {
    let mut paid: HashMap<String, Vec<Movement>> = HashMap::new();
    for movement in payments(connection, "loan_id")? {
        if let Some(loan) = movement.loan_id.clone() {
            paid.entry(loan).or_default().push(movement);
        }
    }
    Ok(list_loans(connection)?
        .into_iter()
        .map(|loan| {
            let payments = paid.remove(&loan.id).unwrap_or_default();
            loan_state(loan, &payments)
        })
        .collect())
}

fn loan_state(loan: Loan, payments: &[Movement]) -> LoanState {
    let installments = (1..=loan.installment_count)
        .map(|number| LoanInstallment {
            number,
            month: add_months(&loan.first_month, number as i32 - 1),
            paid: payments.iter().find(|movement| movement.installment_number == Some(number)).map(Payment::of),
        })
        .collect::<Vec<_>>();
    let paid = payments.iter().filter(|movement| movement.installment_number.is_some_and(|number| number <= loan.installment_count)).collect::<Vec<_>>();
    let total_paid: i128 = paid.iter().map(|movement| cents(&movement.amount)).sum();
    let last_paid = paid.iter().max_by(|a, b| (&a.date, &a.created_at).cmp(&(&b.date, &b.created_at))).map(|movement| movement.amount.clone());
    let next_number = installments.iter().find(|installment| installment.paid.is_none()).map(|installment| installment.number);
    LoanState {
        paid_count: paid.len() as u32,
        total_paid: crate::finance::format_cents(total_paid),
        last_paid,
        finished: next_number.is_none(),
        next_number,
        installments,
        loan,
    }
}

pub(crate) fn loan_state_of(connection: &Connection, id: &str) -> ReadResult<Option<LoanState>> {
    let Some(loan) = load_loan(connection, id)? else { return Ok(None) };
    let payments = rows(
        connection,
        &format!("SELECT {MOVEMENT_COLUMNS} FROM finance_movements WHERE loan_id=?1 ORDER BY date, created_at"),
        [id],
        movement_from,
    )?;
    Ok(Some(loan_state(loan, &payments)))
}

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonth {
    #[serde(flatten)]
    pub service: Service,
    /// The payment of the month, if there is one.
    pub paid: Option<Payment>,
    /// What it should cost: the last payment, or the amount of the service.
    pub estimate: String,
}

/// Each service in `month`.
pub(crate) fn services_of_month(connection: &Connection, month: &str) -> ReadResult<Vec<ServiceMonth>> {
    let all = payments(connection, "service_id")?;
    Ok(list_services(connection)?
        .into_iter()
        .map(|service| {
            let of_service = all.iter().filter(|movement| movement.service_id.as_deref() == Some(service.id.as_str())).collect::<Vec<_>>();
            let paid = of_service.iter().find(|movement| movement.service_month.as_deref() == Some(month)).map(|movement| Payment::of(movement));
            let estimate = of_service.last().map(|movement| movement.amount.clone()).unwrap_or_else(|| service.amount.clone());
            ServiceMonth { service, paid, estimate }
        })
        .collect())
}

/// What a service should cost now: its last payment, or its amount.
pub(crate) fn service_estimate(connection: &Connection, service: &Service) -> ReadResult<String> {
    let last: Option<String> = connection
        .query_row(
            "SELECT amount FROM finance_movements WHERE service_id=?1 ORDER BY date DESC, created_at DESC LIMIT 1",
            [&service.id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(last.unwrap_or_else(|| service.amount.clone()))
}

// ---------------------------------------------------------------------------
// Savings
// ---------------------------------------------------------------------------

/// What a reserve holds: deposits minus withdrawals.
pub(crate) fn reserve_balances(connection: &Connection) -> ReadResult<HashMap<String, i128>> {
    let mut balances = HashMap::new();
    for movement in list_savings_movements(connection)? {
        let amount = cents(&movement.amount);
        *balances.entry(movement.reserve_id).or_insert(0) += if movement.kind == "deposit" { amount } else { -amount };
    }
    Ok(balances)
}

/// What went to savings in each currency between two dates, net of what
/// came back: a deposit or withdrawal counts in the reserve's currency, and
/// buying or selling dollars counts the pesos.
pub(crate) fn saved_between(connection: &Connection, from: &str, to: &str) -> ReadResult<HashMap<String, i128>> {
    let reserves = list_reserves(connection)?.into_iter().map(|reserve| (reserve.id, reserve.currency)).collect::<HashMap<_, _>>();
    let mut saved: HashMap<String, i128> = HashMap::new();
    for movement in list_savings_movements(connection)?.into_iter().filter(|movement| movement.date.as_str() >= from && movement.date.as_str() <= to) {
        let sign = if movement.kind == "deposit" { 1 } else { -1 };
        match movement.pesos.as_deref() {
            Some(pesos) => *saved.entry("ARS".into()).or_default() += sign * cents(pesos),
            None => {
                let currency = reserves.get(&movement.reserve_id).cloned().unwrap_or_else(|| "ARS".into());
                *saved.entry(currency).or_default() += sign * cents(&movement.amount);
            }
        }
    }
    Ok(saved)
}
