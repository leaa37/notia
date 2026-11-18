//! What the Finanzas screen shows, already derived from the records: the
//! month overview, its movements grouped by account, the products with their
//! prices and the salary and savings tab. The screen formats these figures
//! and lays them out; every rule that decides them lives here.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::finance::{format_cents, parse_cents, validate_context, FinanceCommandResult, FinanceContext};
use crate::finance_matching::{normalize_text, ReviewItem, CARD_UNPAID};
use crate::finance_reconciliation::previous_period;

const MONTHS: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];
const SHORT_MONTHS: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];
/// A card statement "cuadra" when its lines add up to its total within a peso.
const STATEMENT_TOLERANCE_CENTS: i128 = 100;
const LATEST_MOVEMENTS: usize = 5;
const LARGEST_EXPENSES: usize = 4;
const SERVICE_HISTORY_MONTHS: i64 = 6;
const SALARY_BARS: usize = 12;
const SHARE_MONTHS: i64 = 12;
const TICKET_LIMIT: usize = 50;

// ---------------------------------------------------------------------------
// Amounts, dates and wording
// ---------------------------------------------------------------------------

fn cents(value: &str) -> i128 {
    parse_cents(value).unwrap_or(0)
}

fn counted(status: &str) -> bool {
    matches!(status, "confirmed" | "corrected")
}

/// An amount in one currency, as a decimal string with two decimals.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub currency: String,
    pub amount: String,
}

fn money(currency: &str, amount: i128) -> Money {
    Money { currency: currency.to_string(), amount: format_cents(amount) }
}

type Totals = BTreeMap<String, i128>;

fn add(totals: &mut Totals, currency: &str, amount: i128) {
    *totals.entry(currency.to_string()).or_default() += amount;
}

/// Each currency apart, pesos first; pesos and dollars never add up.
fn money_list(totals: &Totals) -> Vec<Money> {
    totals.iter().filter(|(_, amount)| **amount != 0).map(|(currency, amount)| money(currency, *amount)).collect()
}

/// Share of `part` in `whole`, in percent with one decimal (floored, as the
/// card ratios always were).
fn percent(part: i128, whole: i128) -> Option<f64> {
    (whole > 0).then(|| ((part * 1_000) / whole) as f64 / 10.0)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Price of one unit of `bought` paid with `paid`, with two decimals.
fn rate(paid: i128, bought: i128) -> Option<String> {
    (bought > 0).then(|| format_cents(paid * 100 / bought))
}

fn thousands(value: i128) -> String {
    let digits = value.abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    if value < 0 { format!("-{grouped}") } else { grouped }
}

/// Money inside a sentence: whole units, `$ 90.000` or `USD 1.000`.
fn spoken_money(amount: i128, currency: &str) -> String {
    let units = (amount + if amount < 0 { -50 } else { 50 }) / 100;
    let prefix = if currency == "ARS" { "$" } else { currency };
    format!("{prefix} {}", thousands(units))
}

fn month_index(period: &str) -> Option<usize> {
    let month = period.get(5..7)?.parse::<usize>().ok()?;
    (1..=12).contains(&month).then(|| month - 1)
}

/// `septiembre de 2026`.
fn month_label(period: &str) -> String {
    match (month_index(period), period.get(..4)) {
        (Some(index), Some(year)) => format!("{} de {year}", MONTHS[index]),
        _ => period.to_string(),
    }
}

/// `5 ago`.
fn short_date(date: &str) -> String {
    let day = date.get(8..10).and_then(|day| day.parse::<u32>().ok());
    match (day, month_index(date)) {
        (Some(day), Some(index)) => format!("{day} {}", SHORT_MONTHS[index]),
        _ => date.to_string(),
    }
}

fn shift_month(period: &str, offset: i64) -> Option<String> {
    notia_backend_core::finance_insights::shift_month(period, offset)
}

fn valid_month(period: &str) -> Result<(), String> {
    if crate::finance::valid_service_period(period) {
        Ok(())
    } else {
        Err("El mes debe tener formato YYYY-MM.".into())
    }
}

fn storage(error: rusqlite::Error) -> String {
    error.to_string()
}

// ---------------------------------------------------------------------------
// Accounts and statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Account {
    account_type: String,
    /// `Mastercard ••0-3` for a card with a loaded statement.
    label: String,
    badge: String,
}

fn card_badge(text: &str) -> &'static str {
    let text = text.to_lowercase();
    if text.contains("master") {
        "MC"
    } else if text.contains("visa") {
        "VISA"
    } else if text.contains("amex") || text.contains("american") {
        "AMEX"
    } else if text.contains("naranja") {
        "NX"
    } else if text.contains("cabal") {
        "CABAL"
    } else {
        "TC"
    }
}

fn account_kind_label(account_type: &str) -> &'static str {
    match account_type {
        "credit_card" => "Tarjeta de crédito",
        "bank" => "Cuenta bancaria",
        "cash" => "Efectivo",
        "wallet" => "Billetera virtual",
        "savings_reserve" => "Reserva de ahorro",
        _ => "Cuenta",
    }
}

fn load_accounts(connection: &Connection) -> Result<HashMap<String, Account>, String> {
    let mut cards = HashMap::<String, (String, Option<String>)>::new();
    let mut statement = connection
        .prepare("SELECT account_id,issuer,card_last_four FROM finance_credit_card_statements ORDER BY due_date DESC")
        .map_err(storage)?;
    for row in statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))
        .map_err(storage)?
    {
        let (account_id, issuer, last_four) = row.map_err(storage)?;
        cards.entry(account_id).or_insert((issuer, last_four));
    }
    let mut statement = connection.prepare("SELECT id,name,account_type,currency FROM finance_accounts").map_err(storage)?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .map(|(id, name, account_type, currency)| {
            let card = (account_type == "credit_card").then(|| cards.get(&id)).flatten();
            let label = match card {
                Some((issuer, Some(last_four))) if !last_four.trim().is_empty() => format!("{} ••{}", issuer.trim(), last_four.trim()),
                Some((issuer, _)) if !issuer.trim().is_empty() => issuer.trim().to_string(),
                _ => name.clone(),
            };
            let badge = if account_type == "credit_card" {
                card_badge(&format!("{} {name}", card.map_or("", |(issuer, _)| issuer.as_str()))).to_string()
            } else {
                currency.clone()
            };
            (id, Account { account_type, label, badge })
        })
        .collect())
}

fn account_label(accounts: &HashMap<String, Account>, id: &str) -> String {
    accounts.get(id).map_or_else(|| "Cuenta no disponible".to_string(), |account| account.label.clone())
}

#[derive(Debug, Clone)]
struct StatementLine {
    item_type: String,
    amount: i128,
    description: String,
    transaction_status: Option<String>,
}

#[derive(Debug, Clone)]
struct Statement {
    id: String,
    account_id: String,
    due_date: String,
    currency: String,
    previous_balance: i128,
    payments: i128,
    total_due: i128,
    lines: Vec<StatementLine>,
}

fn load_statements_due(connection: &Connection, month: &str) -> Result<Vec<Statement>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,account_id,due_date,currency,previous_balance,payments_amount,total_due
             FROM finance_credit_card_statements WHERE substr(due_date,1,7)=?1",
        )
        .map_err(storage)?;
    let mut statements = statement
        .query_map([month], |row| {
            Ok(Statement {
                id: row.get(0)?,
                account_id: row.get(1)?,
                due_date: row.get(2)?,
                currency: row.get(3)?,
                previous_balance: cents(&row.get::<_, String>(4)?),
                payments: cents(&row.get::<_, String>(5)?),
                total_due: cents(&row.get::<_, String>(6)?),
                lines: Vec::new(),
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut lines = connection
        .prepare(
            "SELECT i.item_type,i.amount,i.description,t.status FROM finance_credit_card_statement_items i
             LEFT JOIN finance_transactions t ON t.id=i.transaction_id AND t.deleted_at IS NULL
             WHERE i.statement_id=?1",
        )
        .map_err(storage)?;
    for statement in &mut statements {
        statement.lines = lines
            .query_map([&statement.id], |row| {
                Ok(StatementLine {
                    item_type: row.get(0)?,
                    amount: cents(&row.get::<_, String>(1)?),
                    description: row.get(2)?,
                    transaction_status: row.get(3)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
    }
    statements.sort_by(|left, right| right.total_due.cmp(&left.total_due));
    Ok(statements)
}

/// What the loaded lines of a statement add up to: purchases, fees,
/// interest and taxes, minus credits. Payment lines pay the previous balance.
fn lines_total(statement: &Statement) -> i128 {
    statement
        .lines
        .iter()
        .map(|line| match line.item_type.as_str() {
            "credit" => -line.amount,
            "payment" => 0,
            _ => line.amount,
        })
        .sum()
}

/// The part of the total that new lines explain: the total minus what was
/// left unpaid from the previous statement.
fn expected_lines(statement: &Statement) -> i128 {
    statement.total_due - (statement.previous_balance - statement.payments).max(0)
}

// ---------------------------------------------------------------------------
// Review items
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPart {
    text: String,
    strong: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAction {
    label: String,
    /// Message the chat composer receives; nothing changes until the
    /// person sends it and confirms what the assistant proposes.
    prompt: String,
    primary: bool,
    /// The option `finance_resolve_review` applies at once, without the
    /// chat; `None` when the doubt is only answered in the chat.
    option_id: Option<String>,
}

/// A doubt the screen shows in «Para revisar»; it is answered in the chat.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCard {
    id: String,
    label: String,
    parts: Vec<TextPart>,
    actions: Vec<ReviewAction>,
    /// The doubt as a request for the chat, without choosing an answer:
    /// what Home sends with «Revisar en el chat».
    #[serde(skip)]
    ask: String,
}

fn text(value: impl Into<String>) -> TextPart {
    TextPart { text: value.into(), strong: false }
}

fn strong(value: impl Into<String>) -> TextPart {
    TextPart { text: value.into(), strong: true }
}

/// Text between «» is what the question is about: it is shown in bold.
fn question_parts(question: &str) -> Vec<TextPart> {
    let mut parts = Vec::new();
    let mut rest = question;
    while let Some(start) = rest.find('«') {
        let Some(length) = rest[start..].find('»') else { break };
        if start > 0 {
            parts.push(text(&rest[..start]));
        }
        parts.push(strong(&rest[start + '«'.len_utf8()..start + length]));
        rest = &rest[start + length + '»'.len_utf8()..];
    }
    if !rest.is_empty() {
        parts.push(text(rest));
    }
    parts
}

fn review_label(kind: &str) -> &'static str {
    match kind {
        "service-card-line" => "Servicio",
        "card-line-candidates" | "unpaid-line-candidates" => "Tarjeta",
        "similar-merchant" => "Comercio",
        "similar-product" => "Producto",
        "missing-installment" => "Cuotas",
        _ => "Para revisar",
    }
}

fn review_card(item: &ReviewItem) -> ReviewCard {
    let mut actions = item
        .options
        .iter()
        .enumerate()
        .map(|(index, option)| ReviewAction {
            label: option.label.clone(),
            prompt: format!("Sobre la duda «{}» (caso {}): {}.", item.question, item.id, option.label),
            primary: index == 0,
            option_id: Some(option.id.clone()),
        })
        .collect::<Vec<_>>();
    if actions.is_empty() {
        actions.push(ReviewAction {
            label: "Responder en el chat".into(),
            prompt: format!("Sobre la duda «{}» (caso {}): ", item.question, item.id),
            primary: true,
            option_id: None,
        });
    }
    ReviewCard {
        id: item.id.clone(),
        label: review_label(&item.kind).into(),
        parts: question_parts(&item.question),
        actions,
        ask: format!("Revisemos la duda de Finanzas «{}» (caso {}).", item.question, item.id),
    }
}

/// One request with every doubt of «Para revisar», for the chat.
fn review_request(cards: &[ReviewCard]) -> Option<String> {
    match cards {
        [] => None,
        [card] => Some(card.ask.clone()),
        cards => Some(format!(
            "Revisemos lo que quedó para revisar en Finanzas:\n{}",
            cards.iter().map(|card| format!("- {}", card.ask)).collect::<Vec<_>>().join("\n")
        )),
    }
}

fn pending_reviews(connection: &Connection) -> Result<Vec<ReviewItem>, String> {
    crate::finance_matching::list_review_items(connection, Some("pending"), 50)
}

/// Transaction a review item asks about, directly or through its card line.
fn reviews_by_transaction(connection: &Connection, reviews: &[ReviewItem]) -> Result<HashMap<String, String>, String> {
    let mut by_transaction = HashMap::new();
    for review in reviews {
        let transaction = match (review.subject["transactionId"].as_str(), review.subject["lineId"].as_str()) {
            (Some(id), _) => Some(id.to_string()),
            (None, Some(line_id)) => connection
                .query_row("SELECT transaction_id FROM finance_credit_card_statement_items WHERE id=?1", [line_id], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .ok()
                .flatten(),
            _ => None,
        };
        if let Some(transaction) = transaction {
            by_transaction.entry(transaction).or_insert_with(|| review.question.clone());
        }
    }
    Ok(by_transaction)
}

// ---------------------------------------------------------------------------
// Movements of a month
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeDetail {
    /// What went into the savings reserve.
    amount: String,
    currency: String,
    reserve_name: String,
    /// Price paid per unit of the reserve's currency.
    rate: Option<String>,
    /// `true` when it bought savings; `false` when it sold them.
    into_savings: bool,
}

/// One movement as the lists and the detail panel show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementRow {
    id: String,
    /// `expense`, `income`, `exchange`, `transfer` or `adjustment`.
    kind: String,
    status: String,
    description: String,
    /// Day of the purchase (or of the movement when there is no purchase).
    purchase_date: String,
    /// Day it counts: for a card expense, the day its statement is due.
    effective_date: String,
    counts_on_statement_due: bool,
    account_id: String,
    account_name: String,
    category_name: Option<String>,
    uncategorized: bool,
    service_name: Option<String>,
    origin: String,
    via_telegram: bool,
    amount: String,
    currency: String,
    exchange: Option<ExchangeDetail>,
    /// It has a pending question in «Para revisar».
    flagged: bool,
    note: Option<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    row: MovementRow,
    destination: Option<String>,
    category_id: Option<String>,
    amount: i128,
    to_savings: bool,
    search: String,
}

impl Entry {
    fn counted_expense(&self) -> bool {
        self.row.kind == "expense" && counted(&self.row.status)
    }
}

fn origin_label(source: &str, artifact_type: Option<&str>) -> String {
    let label = match (source, artifact_type) {
        ("credit_card_statement", _) | (_, Some("credit_card_statement")) => "Resumen de tarjeta",
        ("savings_exchange" | "savings", _) => "Ahorro",
        ("installment", _) => "Plan de cuotas",
        (_, Some("ticket")) => "Ticket",
        (_, Some("salary")) => "Recibo de sueldo",
        ("telegram", _) => "Telegram",
        ("app", _) => "Chat de Notia",
        ("public-url", _) => "URL pública",
        ("manual", _) => "Carga manual",
        ("android", _) => "App Notia Finanzas",
        _ => "Asistente",
    };
    if source == "telegram" && label != "Telegram" {
        format!("{label} por Telegram")
    } else {
        label.to_string()
    }
}

struct Linked {
    amount: i128,
    currency: String,
    reserve_name: String,
    movement_type: String,
}

fn linked_savings(connection: &Connection) -> Result<HashMap<String, Linked>, String> {
    let mut statement = connection
        .prepare(
            "SELECT m.linked_transaction_id,m.amount,m.currency,COALESCE(r.name,''),m.movement_type
             FROM finance_savings_movements m LEFT JOIN finance_savings_reserves r ON r.id=m.reserve_id
             WHERE m.linked_transaction_id IS NOT NULL",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Linked {
                    amount: cents(&row.get::<_, String>(1)?),
                    currency: row.get(2)?,
                    reserve_name: row.get(3)?,
                    movement_type: row.get(4)?,
                },
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows.into_iter().collect())
}

fn movement_note(status: &str, flagged: Option<&String>, statement_name: Option<&str>) -> Option<String> {
    match status {
        "discarded" => Some(match statement_name {
            Some(name) => format!("Descartado: no suma en los gastos del mes, aunque sigue incluido en el total del resumen de {name}."),
            None => "Descartado: no suma en los gastos del mes.".to_string(),
        }),
        CARD_UNPAID => Some("En tarjeta, a pagar: va a contar en el mes en que venza el resumen que lo incluya.".into()),
        _ => match flagged {
            Some(question) => Some(format!("{question} Está en «Para revisar».")),
            None if status == "pending" => Some("Pendiente de confirmar: todavía no suma en los gastos.".into()),
            None => None,
        },
    }
}

struct MonthLedger {
    accounts: HashMap<String, Account>,
    entries: Vec<Entry>,
}

fn load_month(connection: &Connection, month: &str, reviews: &[ReviewItem]) -> Result<MonthLedger, String> {
    let (from, to) = notia_backend_core::finance_insights::month_range(month).ok_or("El mes no es válido.")?;
    load_range(connection, &from, &to, reviews)
}

/// The movements that count between two days (`YYYY-MM-DD`, both in), the
/// most recent first.
fn load_range(connection: &Connection, from: &str, to: &str, reviews: &[ReviewItem]) -> Result<MonthLedger, String> {
    let accounts = load_accounts(connection)?;
    let categories = connection
        .prepare("SELECT id,name FROM finance_categories")
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(storage)?;
    let services = connection
        .prepare("SELECT id,name FROM finance_services")
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(storage)?;
    let merchants = connection
        .prepare("SELECT id,name FROM finance_merchants")
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(storage)?;
    let linked = linked_savings(connection)?;
    let flagged = reviews_by_transaction(connection, reviews)?;
    let mut statement = connection
        .prepare(
            "SELECT t.id,t.transaction_type,t.amount,t.currency,substr(t.effective_date,1,10),t.purchase_date,t.account_id,
                    t.destination_account_id,t.category_id,t.description,t.source,t.status,t.service_id,t.merchant_id,
                    a.source_type,s.account_id
             FROM finance_transactions t
             LEFT JOIN finance_source_artifacts a ON a.id=t.source_artifact_id
             LEFT JOIN finance_credit_card_statement_items i ON i.transaction_id=t.id
             LEFT JOIN finance_credit_card_statements s ON s.id=i.statement_id
             WHERE t.deleted_at IS NULL AND substr(t.effective_date,1,10) BETWEEN ?1 AND ?2
             ORDER BY COALESCE(t.purchase_date,t.effective_date) DESC,t.created_at DESC",
        )
        .map_err(storage)?;
    type Raw = (
        String, String, String, String, String, Option<String>, String, Option<String>,
        Option<String>, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>,
    );
    let rows = statement
        .query_map(params![from, to], |row| {
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?, row.get(15)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<Raw>, _>>()
        .map_err(storage)?;
    let entries = rows
        .into_iter()
        .map(|(id, kind, amount, currency, effective, purchase, account_id, destination, category_id, description, source, status, service_id, merchant_id, artifact_type, statement_account)| {
            let category_id = category_id.filter(|id| !id.is_empty());
            let category_name = category_id.as_ref().map(|id| categories.get(id).cloned().unwrap_or_else(|| "Categoría inexistente".into()));
            let service_name = service_id.as_ref().and_then(|id| services.get(id).cloned());
            let account_name = account_label(&accounts, &account_id);
            let statement_name = statement_account.as_deref().map(|id| account_label(&accounts, id));
            let exchange = linked.get(&id).filter(|_| kind == "exchange").map(|link| ExchangeDetail {
                amount: format_cents(link.amount),
                currency: link.currency.clone(),
                reserve_name: link.reserve_name.clone(),
                rate: if link.currency != currency { rate(cents(&amount), link.amount) } else { None },
                into_savings: link.movement_type == "contribution",
            });
            let to_savings = kind == "exchange"
                || (kind == "transfer"
                    && destination.as_deref().and_then(|id| accounts.get(id)).is_some_and(|account| account.account_type == "savings_reserve"));
            let purchase_date = purchase.filter(|date| !date.is_empty()).unwrap_or_else(|| effective.clone());
            let merchant = merchant_id.as_ref().and_then(|id| merchants.get(id)).cloned().unwrap_or_default();
            let search = normalize_text(&format!(
                "{description} {account_name} {} {} {merchant}",
                category_name.as_deref().unwrap_or("sin categoria"),
                service_name.as_deref().unwrap_or_default()
            ));
            let flag = flagged.get(&id).filter(|_| status != "discarded");
            Entry {
                amount: cents(&amount),
                destination: destination.clone(),
                row: MovementRow {
                    uncategorized: kind == "expense" && category_id.is_none(),
                    counts_on_statement_due: statement_name.is_some() && purchase_date != effective,
                    note: movement_note(&status, flag, statement_name.as_deref()),
                    flagged: flag.is_some(),
                    origin: origin_label(&source, artifact_type.as_deref()),
                    via_telegram: source == "telegram",
                    id,
                    kind,
                    status,
                    description,
                    purchase_date,
                    effective_date: effective,
                    account_id,
                    account_name,
                    category_name,
                    service_name,
                    amount,
                    currency,
                    exchange,
                },
                category_id,
                to_savings,
                search,
            }
        })
        .collect();
    Ok(MonthLedger { accounts, entries })
}

fn matches_filter(entry: &Entry, filter: &str) -> bool {
    match filter {
        "" | "all" => true,
        "uncategorized" => entry.counted_expense() && entry.category_id.is_none(),
        "savings" => entry.to_savings,
        "income" => entry.row.kind == "income",
        "card-unpaid" => entry.row.status == CARD_UNPAID,
        "pending" => entry.row.status == "pending",
        "discarded" => entry.row.status == "discarded",
        other => other
            .strip_prefix("category:")
            .is_some_and(|category| entry.counted_expense() && entry.category_id.as_deref() == Some(category)),
    }
}

fn matches_search(entry: &Entry, search: &str) -> bool {
    let search = normalize_text(search);
    search.split_whitespace().all(|word| entry.search.contains(word))
}

// ---------------------------------------------------------------------------
// Month flows over the salary that paid them
// ---------------------------------------------------------------------------

/// What left the salary in a month, in one currency, and whether anything
/// was recorded for each part.
#[derive(Debug, Default, Clone, Copy)]
struct Flows {
    cards: (i128, bool),
    services_apart: (i128, bool),
    savings: (i128, bool),
}

fn sum_query(connection: &Connection, sql: &str, month: &str, currency: &str) -> Result<(i128, bool), String> {
    let mut statement = connection.prepare(sql).map_err(storage)?;
    let amounts = statement
        .query_map(params![month, currency], |row| row.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok((amounts.iter().map(|amount| cents(amount)).sum(), !amounts.is_empty()))
}

fn month_flows(connection: &Connection, month: &str, currency: &str) -> Result<Flows, String> {
    let payment_month = crate::finance::SERVICE_PAYMENT_MONTH;
    let cards = sum_query(
        connection,
        "SELECT total_due FROM finance_credit_card_statements WHERE substr(due_date,1,7)=?1 AND currency=?2",
        month,
        currency,
    )?;
    // Services paid by card are already inside the card total.
    let services_apart = sum_query(
        connection,
        &format!(
            "SELECT o.paid_amount FROM finance_service_occurrences o
             JOIN finance_services s ON s.id=o.service_id
             LEFT JOIN finance_transactions t ON t.id=o.transaction_id AND t.deleted_at IS NULL
             LEFT JOIN finance_accounts a ON a.id=t.account_id
             WHERE o.paid_amount IS NOT NULL AND s.currency=?2 AND {payment_month}=?1
               AND (a.account_type IS NULL OR a.account_type<>'credit_card')"
        ),
        month,
        currency,
    )?;
    let bought = sum_query(
        connection,
        "SELECT t.amount FROM finance_transactions t
         JOIN finance_savings_movements m ON m.linked_transaction_id=t.id AND m.movement_type='contribution'
         WHERE t.transaction_type='exchange' AND t.deleted_at IS NULL AND t.status IN ('confirmed','corrected')
           AND substr(t.effective_date,1,7)=?1 AND t.currency=?2",
        month,
        currency,
    )?;
    let contributed = sum_query(
        connection,
        "SELECT m.amount FROM finance_savings_movements m
         WHERE m.movement_type='contribution' AND m.status IN ('confirmed','corrected')
           AND substr(m.effective_date,1,7)=?1 AND m.currency=?2
           AND NOT EXISTS (SELECT 1 FROM finance_transactions t WHERE t.id=m.linked_transaction_id AND t.transaction_type='exchange')",
        month,
        currency,
    )?;
    Ok(Flows { cards, services_apart, savings: (bought.0 + contributed.0, bought.1 || contributed.1) })
}

struct SalaryReceipts {
    currency: String,
    net: i128,
    employers: Vec<String>,
    payment_date: Option<String>,
}

/// Net salary of a period in its main currency (pesos when there are).
fn salary_of(connection: &Connection, period: &str) -> Result<Option<SalaryReceipts>, String> {
    let mut statement = connection
        .prepare("SELECT currency,net_amount,COALESCE(employer,''),payment_date FROM finance_salary_receipts WHERE period=?1 ORDER BY payment_date")
        .map_err(storage)?;
    let rows = statement
        .query_map([period], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let Some(currency) = rows.iter().map(|row| row.0.as_str()).find(|currency| *currency == "ARS").or(rows.first().map(|row| row.0.as_str())) else {
        return Ok(None);
    };
    let currency = currency.to_string();
    let mine = rows.into_iter().filter(|row| row.0 == currency).collect::<Vec<_>>();
    let mut employers = Vec::new();
    for (_, _, employer, _) in &mine {
        if !employer.trim().is_empty() && !employers.contains(employer) {
            employers.push(employer.clone());
        }
    }
    Ok(Some(SalaryReceipts {
        net: mine.iter().map(|row| cents(&row.1)).sum(),
        payment_date: mine.iter().filter_map(|row| row.3.clone()).max(),
        employers,
        currency,
    }))
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthPayload {
    context: FinanceContext,
    month: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalarySegment {
    /// `cards`, `savings`, `services` or `unregistered`.
    key: &'static str,
    amount: String,
    percent: Option<f64>,
}

/// Where the salary that pays the month went.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalaryUse {
    period: String,
    payment_date: Option<String>,
    employers: Vec<String>,
    currency: String,
    net: String,
    segments: Vec<SalarySegment>,
    /// Currency bought for savings with this salary (USD 1.000).
    savings_bought: Vec<Money>,
    /// What was recorded exceeds the salary.
    exceeded: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthExpenses {
    totals: Vec<Money>,
    count: usize,
    /// Card expenses no loaded statement includes yet, whatever their month.
    card_unpaid: Vec<Money>,
    card_unpaid_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveBalance {
    id: String,
    name: String,
    currency: String,
    balance: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedThisMonth {
    contributions: Vec<Money>,
    /// Savings bought with another currency, and what they cost in it.
    bought: Vec<Money>,
    cost: Vec<Money>,
    withdrawals: Vec<Money>,
    /// Price paid per unit when one currency bought another.
    rate: Option<String>,
    reserves: Vec<ReserveBalance>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRow {
    /// `category:<id>` or `uncategorized`: the movements filter of the row.
    filter: String,
    name: String,
    description: Option<String>,
    currency: String,
    amount: String,
    count: usize,
    percent: Option<f64>,
    /// Change against the previous month; `None` without a base.
    variation: Option<f64>,
    uncategorized: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryBreakdown {
    rows: Vec<CategoryRow>,
    previous_month: String,
    has_previous: bool,
    uncategorized_count: usize,
    /// Prompt to categorize them in the chat.
    categorize_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRow {
    id: String,
    account_id: String,
    badge: String,
    name: String,
    amount: String,
    currency: String,
    due_date: String,
    line_count: usize,
    /// `matches`, `missing` (lines add up to less) or `extra` (to more).
    check: &'static str,
    /// Absolute difference between the total and its lines.
    difference: String,
    discarded: Vec<Money>,
    discarded_descriptions: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallmentsSummary {
    pending_count: usize,
    remaining: Vec<Money>,
    next_due_date: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsPaid {
    totals: Vec<Money>,
    statements: Vec<StatementRow>,
    installments: InstallmentsSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRow {
    service_id: String,
    name: String,
    provider: Option<String>,
    currency: String,
    expected: String,
    paid: Option<String>,
    /// `paid`, `pending` or `not-applicable`.
    status: &'static str,
    note: Option<String>,
    /// Status of each month of `historyPeriods`: `paid`, `not-applicable`,
    /// `pending` or `missing` when nothing was recorded.
    history: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesOfMonth {
    pending_count: usize,
    rows: Vec<ServiceRow>,
    history_periods: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceOverview {
    month: String,
    review: Vec<ReviewCard>,
    salary: Option<SalaryUse>,
    /// What came in, what went out and what was left, per currency.
    balance: MonthBalance,
    budgets: Vec<crate::finance_budgets::BudgetRow>,
    loans: crate::finance_loans::LoansOfMonth,
    expenses: MonthExpenses,
    saved: SavedThisMonth,
    categories: CategoryBreakdown,
    largest: Vec<MovementRow>,
    cards: CardsPaid,
    services: ServicesOfMonth,
    latest: Vec<MovementRow>,
    movement_count: usize,
}

fn salary_use(connection: &Connection, month: &str) -> Result<Option<SalaryUse>, String> {
    let Some(period) = previous_period(month) else { return Ok(None) };
    let Some(salary) = salary_of(connection, &period)? else { return Ok(None) };
    let flows = month_flows(connection, month, &salary.currency)?;
    let loans = loans_paid_apart(connection, month, &salary.currency)?;
    let recorded = flows.cards.0 + flows.savings.0 + flows.services_apart.0 + loans;
    let unregistered = (salary.net - recorded).max(0);
    let segment = |key, amount: i128| SalarySegment { key, amount: format_cents(amount), percent: percent(amount, salary.net) };
    let mut bought = Totals::new();
    let mut statement = connection
        .prepare(
            "SELECT m.currency,m.amount FROM finance_savings_movements m
             JOIN finance_transactions t ON t.id=m.linked_transaction_id AND t.transaction_type='exchange'
               AND t.deleted_at IS NULL AND t.status IN ('confirmed','corrected')
             WHERE m.movement_type='contribution' AND substr(t.effective_date,1,7)=?1 AND t.currency=?2",
        )
        .map_err(storage)?;
    for row in statement
        .query_map(params![month, salary.currency], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
    {
        let (currency, amount) = row.map_err(storage)?;
        add(&mut bought, &currency, cents(&amount));
    }
    Ok(Some(SalaryUse {
        segments: vec![
            segment("cards", flows.cards.0),
            segment("savings", flows.savings.0),
            segment("services", flows.services_apart.0),
            segment("loans", loans),
            segment("unregistered", unregistered),
        ],
        exceeded: recorded > salary.net,
        savings_bought: money_list(&bought),
        net: format_cents(salary.net),
        period,
        payment_date: salary.payment_date,
        employers: salary.employers,
        currency: salary.currency,
    }))
}

/// Loan installments paid in `month` from an account that is not a card:
/// what a card paid is already inside its statement.
fn loans_paid_apart(connection: &Connection, month: &str, currency: &str) -> Result<i128, String> {
    let amounts = connection
        .prepare(
            "SELECT t.amount FROM finance_loan_installments i
             JOIN finance_transactions t ON t.id=i.transaction_id AND t.deleted_at IS NULL AND t.status IN ('confirmed','corrected')
             JOIN finance_accounts a ON a.id=t.account_id
             WHERE a.account_type<>'credit_card' AND substr(t.effective_date,1,7)=?1 AND t.currency=?2",
        )
        .map_err(storage)?
        .query_map(params![month, currency], |row| row.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(amounts.iter().map(|amount| cents(amount)).sum())
}

/// One currency of the month's balance.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRow {
    currency: String,
    /// Every income that counts: the salary and the rest.
    income: String,
    salary: String,
    other_income: String,
    expenses: String,
    /// Net that went to savings: bought or moved in, minus sold or taken out.
    savings: String,
    /// Income minus expenses minus savings.
    left: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthBalance {
    rows: Vec<BalanceRow>,
    /// The incomes that are not the salary, the largest first.
    other_incomes: Vec<MovementRow>,
}

const OTHER_INCOMES: usize = 5;

fn month_balance(connection: &Connection, entries: &[Entry]) -> Result<MonthBalance, String> {
    let salaries = connection
        .prepare("SELECT transaction_id FROM finance_salary_receipts WHERE transaction_id IS NOT NULL")
        .map_err(storage)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(storage)?;
    let accounts = load_accounts(connection)?;
    let is_savings = |id: Option<&str>| id.and_then(|id| accounts.get(id)).is_some_and(|account| account.account_type == "savings_reserve");
    let mut figures = BTreeMap::<String, [i128; 4]>::new();
    let mut others = Vec::new();
    for entry in entries.iter().filter(|entry| counted(&entry.row.status)) {
        let row = &entry.row;
        let figure = figures.entry(row.currency.clone()).or_default();
        match row.kind.as_str() {
            "income" if salaries.contains(&row.id) => figure[0] += entry.amount,
            "income" => {
                figure[1] += entry.amount;
                others.push(entry);
            }
            "expense" => figure[2] += entry.amount,
            // A purchase of savings takes the money of its account; a sale
            // brings it back.
            "exchange" => match &row.exchange {
                Some(exchange) if !exchange.into_savings => figure[3] -= entry.amount,
                _ => figure[3] += entry.amount,
            },
            "transfer" => {
                let destination = entry.destination.as_deref();
                match (is_savings(Some(&row.account_id)), is_savings(destination)) {
                    (false, true) => figure[3] += entry.amount,
                    (true, false) => figure[3] -= entry.amount,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let rows = figures
        .into_iter()
        .filter(|(_, figure)| figure.iter().any(|value| *value != 0))
        .map(|(currency, [salary, other, expenses, savings])| BalanceRow {
            currency,
            income: format_cents(salary + other),
            salary: format_cents(salary),
            other_income: format_cents(other),
            expenses: format_cents(expenses),
            savings: format_cents(savings),
            left: format_cents(salary + other - expenses - savings),
        })
        .collect();
    others.sort_by_key(|entry| std::cmp::Reverse(entry.amount));
    Ok(MonthBalance { rows, other_incomes: others.into_iter().take(OTHER_INCOMES).map(|entry| entry.row.clone()).collect() })
}

fn month_expenses(connection: &Connection, entries: &[Entry]) -> Result<MonthExpenses, String> {
    let mut totals = Totals::new();
    let mut count = 0;
    for entry in entries.iter().filter(|entry| entry.counted_expense()) {
        add(&mut totals, &entry.row.currency, entry.amount);
        count += 1;
    }
    let unpaid = connection
        .prepare("SELECT currency,amount FROM finance_transactions WHERE status=?1 AND deleted_at IS NULL AND transaction_type='expense'")
        .map_err(storage)?
        .query_map([CARD_UNPAID], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut card_unpaid = Totals::new();
    for (currency, amount) in &unpaid {
        add(&mut card_unpaid, currency, cents(amount));
    }
    Ok(MonthExpenses { totals: money_list(&totals), count, card_unpaid: money_list(&card_unpaid), card_unpaid_count: unpaid.len() })
}

fn saved_this_month(connection: &Connection, month: &str) -> Result<SavedThisMonth, String> {
    let mut statement = connection
        .prepare(
            "SELECT m.movement_type,m.currency,m.amount,t.currency,t.amount FROM finance_savings_movements m
             LEFT JOIN finance_transactions t ON t.id=m.linked_transaction_id AND t.transaction_type='exchange'
               AND t.deleted_at IS NULL AND t.status IN ('confirmed','corrected')
             WHERE m.status IN ('confirmed','corrected') AND substr(m.effective_date,1,7)=?1",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([month], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let (mut contributions, mut bought, mut cost, mut withdrawals) = (Totals::new(), Totals::new(), Totals::new(), Totals::new());
    for (kind, currency, amount, paid_currency, paid) in rows {
        match kind.as_str() {
            "contribution" => {
                add(&mut contributions, &currency, cents(&amount));
                if let (Some(paid_currency), Some(paid)) = (paid_currency, paid) {
                    add(&mut bought, &currency, cents(&amount));
                    add(&mut cost, &paid_currency, cents(&paid));
                }
            }
            "withdrawal" => add(&mut withdrawals, &currency, cents(&amount)),
            _ => {}
        }
    }
    let single = |totals: &Totals| (totals.len() == 1).then(|| totals.values().next().copied()).flatten();
    let rate = match (single(&cost), single(&bought)) {
        (Some(paid), Some(amount)) if cost.keys().next() != bought.keys().next() => rate(paid, amount),
        _ => None,
    };
    let reserves = crate::finance::finance_list_savings_inner(connection)?
        .into_iter()
        .filter(|reserve| reserve.active)
        .map(|reserve| ReserveBalance { id: reserve.id, name: reserve.name, currency: reserve.currency, balance: reserve.balance })
        .collect();
    Ok(SavedThisMonth { contributions: money_list(&contributions), bought: money_list(&bought), cost: money_list(&cost), withdrawals: money_list(&withdrawals), rate, reserves })
}

fn category_breakdown(connection: &Connection, month: &str, entries: &[Entry]) -> Result<CategoryBreakdown, String> {
    let descriptions = connection
        .prepare("SELECT id,description FROM finance_categories")
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
        .map_err(storage)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(storage)?;
    let previous_month = shift_month(month, -1).unwrap_or_default();
    let mut previous = HashMap::<(Option<String>, String), i128>::new();
    let mut statement = connection
        .prepare(
            "SELECT category_id,currency,amount FROM finance_transactions
             WHERE deleted_at IS NULL AND transaction_type='expense' AND status IN ('confirmed','corrected')
               AND substr(effective_date,1,7)=?1",
        )
        .map_err(storage)?;
    for row in statement
        .query_map([&previous_month], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(storage)?
    {
        let (category, currency, amount) = row.map_err(storage)?;
        *previous.entry((category.filter(|id| !id.is_empty()), currency)).or_default() += cents(&amount);
    }
    let mut totals = Totals::new();
    let mut groups = BTreeMap::<(String, Option<String>), (String, i128, usize)>::new();
    for entry in entries.iter().filter(|entry| entry.counted_expense()) {
        add(&mut totals, &entry.row.currency, entry.amount);
        let group = groups
            .entry((entry.row.currency.clone(), entry.category_id.clone()))
            .or_insert_with(|| (entry.row.category_name.clone().unwrap_or_else(|| "Sin categoría".into()), 0, 0));
        group.1 += entry.amount;
        group.2 += 1;
    }
    let mut rows = groups
        .into_iter()
        .map(|((currency, category_id), (name, amount, count))| {
            let before = previous.get(&(category_id.clone(), currency.clone())).copied().unwrap_or(0);
            CategoryRow {
                filter: category_id.as_ref().map_or_else(|| "uncategorized".to_string(), |id| format!("category:{id}")),
                description: category_id.as_ref().and_then(|id| descriptions.get(id).cloned().flatten()),
                percent: percent(amount, totals.get(&currency).copied().unwrap_or(0)),
                variation: (before > 0).then(|| round1((amount - before) as f64 / before as f64 * 100.0)),
                uncategorized: category_id.is_none(),
                amount: format_cents(amount),
                name,
                currency,
                count,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.currency.cmp(&right.currency).then(cents(&right.amount).cmp(&cents(&left.amount))));
    let uncategorized_count = rows.iter().filter(|row| row.uncategorized).map(|row| row.count).sum::<usize>();
    Ok(CategoryBreakdown {
        categorize_prompt: (uncategorized_count > 0).then(|| {
            format!("Ayudame a categorizar los {uncategorized_count} gastos sin categoría de {}.", month_label(month))
        }),
        has_previous: !previous.is_empty(),
        previous_month,
        uncategorized_count,
        rows,
    })
}

fn statement_rows(statements: &[Statement], accounts: &HashMap<String, Account>) -> Vec<StatementRow> {
    statements
        .iter()
        .map(|statement| {
            let difference = expected_lines(statement) - lines_total(statement);
            let check = if difference.abs() <= STATEMENT_TOLERANCE_CENTS {
                "matches"
            } else if difference > 0 {
                "missing"
            } else {
                "extra"
            };
            let discarded = statement.lines.iter().filter(|line| line.transaction_status.as_deref() == Some("discarded")).collect::<Vec<_>>();
            let mut discarded_totals = Totals::new();
            for line in &discarded {
                add(&mut discarded_totals, &statement.currency, line.amount);
            }
            let account = accounts.get(&statement.account_id);
            StatementRow {
                id: statement.id.clone(),
                account_id: statement.account_id.clone(),
                badge: account.map_or_else(|| "TC".into(), |account| account.badge.clone()),
                name: account_label(accounts, &statement.account_id),
                amount: format_cents(statement.total_due),
                currency: statement.currency.clone(),
                due_date: statement.due_date.clone(),
                line_count: statement.lines.iter().filter(|line| line.item_type != "payment").count(),
                check,
                difference: format_cents(difference.abs()),
                discarded: money_list(&discarded_totals),
                discarded_descriptions: discarded.iter().map(|line| line.description.clone()).collect(),
            }
        })
        .collect()
}

fn statement_reviews(rows: &[StatementRow], statements: &[Statement]) -> Vec<ReviewCard> {
    rows.iter()
        .zip(statements)
        .filter(|(row, _)| row.check != "matches")
        .map(|(row, statement)| {
            let total = spoken_money(statement.total_due, &row.currency);
            let lines = spoken_money(lines_total(statement), &row.currency);
            let prompt = format!(
                "Revisá el resumen de {} que vence el {}: su total es {} {} y las líneas cargadas suman {} {}. ¿Qué falta o sobra?",
                row.name,
                row.due_date,
                row.currency,
                row.amount,
                row.currency,
                format_cents(lines_total(statement))
            );
            ReviewCard {
                id: format!("statement:{}", row.id),
                label: "Resumen de tarjeta".into(),
                parts: vec![
                    text("El resumen de "),
                    strong(&row.name),
                    text(format!(" es de {total}, pero sus líneas cargadas suman {lines}.")),
                ],
                actions: vec![ReviewAction { label: "Revisar en el chat".into(), prompt: prompt.clone(), primary: true, option_id: None }],
                ask: prompt,
            }
        })
        .collect()
}

fn installments_summary(connection: &Connection) -> Result<InstallmentsSummary, String> {
    let rows = connection
        .prepare(
            "SELECT i.amount,p.currency,i.due_date FROM finance_installments i
             JOIN finance_installment_plans p ON p.id=i.plan_id WHERE i.status='pending' ORDER BY i.due_date",
        )
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut remaining = Totals::new();
    for (amount, currency, _) in &rows {
        add(&mut remaining, currency, cents(amount));
    }
    Ok(InstallmentsSummary {
        pending_count: rows.len(),
        next_due_date: rows.first().map(|row| row.2.clone()),
        remaining: money_list(&remaining),
    })
}

/// The pending review that proposes a service for a card line, with the
/// line's date and card.
fn service_line_note(connection: &Connection, reviews: &[ReviewItem], accounts: &HashMap<String, Account>, service_id: &str) -> Option<String> {
    let option = format!("service:{service_id}");
    let review = reviews
        .iter()
        .find(|review| review.kind == "service-card-line" && review.options.iter().any(|candidate| candidate.id == option))?;
    let line_id = review.subject["lineId"].as_str()?;
    let (date, account_id) = connection
        .query_row(
            "SELECT i.purchase_date,s.account_id FROM finance_credit_card_statement_items i
             JOIN finance_credit_card_statements s ON s.id=i.statement_id WHERE i.id=?1",
            [line_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()?;
    Some(format!(
        "Hay un consumo del {} en {} que puede ser este pago. Está en «Para revisar».",
        short_date(&date),
        account_label(accounts, &account_id)
    ))
}

fn services_of_month(connection: &Connection, month: &str, reviews: &[ReviewItem], accounts: &HashMap<String, Account>) -> Result<ServicesOfMonth, String> {
    let history_periods = (0..SERVICE_HISTORY_MONTHS).rev().filter_map(|offset| shift_month(month, -offset)).collect::<Vec<_>>();
    let services = connection
        .prepare("SELECT id,name,provider,currency,expected_amount FROM finance_services WHERE active=1 ORDER BY name")
        .map_err(storage)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut occurrences = HashMap::<(String, String), (String, Option<String>, Option<String>, String)>::new();
    let mut statement = connection
        .prepare(
            "SELECT service_id,period,expected_amount,paid_amount,transaction_id,status FROM finance_service_occurrences
             WHERE period BETWEEN ?1 AND ?2",
        )
        .map_err(storage)?;
    for row in statement
        .query_map(params![history_periods.first(), month], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?))
        })
        .map_err(storage)?
    {
        let (service, period, expected, paid, transaction, status) = row.map_err(storage)?;
        occurrences.insert((service, period), (expected, paid, transaction, status));
    }
    let status_of = |occurrence: Option<&(String, Option<String>, Option<String>, String)>| match occurrence {
        Some((_, paid, transaction, _)) if paid.is_some() || transaction.is_some() => "paid",
        Some((_, _, _, status)) if matches!(status.as_str(), "discarded" | "rejected") => "not-applicable",
        Some(_) => "pending",
        None => "missing",
    };
    let mut rows = services
        .into_iter()
        .map(|(service_id, name, provider, currency, expected)| {
            let occurrence = occurrences.get(&(service_id.clone(), month.to_string()));
            let status = match status_of(occurrence) {
                "missing" => "pending",
                status => status,
            };
            ServiceRow {
                note: (status == "pending").then(|| service_line_note(connection, reviews, accounts, &service_id)).flatten(),
                history: history_periods.iter().map(|period| status_of(occurrences.get(&(service_id.clone(), period.clone())))).collect(),
                expected: format_cents(cents(occurrence.map_or(&expected, |occurrence| &occurrence.0))),
                paid: occurrence.and_then(|occurrence| occurrence.1.as_deref()).map(|paid| format_cents(cents(paid))),
                service_id,
                name,
                provider: provider.filter(|provider| !provider.trim().is_empty()),
                currency,
                status,
            }
        })
        .collect::<Vec<_>>();
    let rank = |status: &str| match status {
        "pending" => 0,
        "paid" => 1,
        _ => 2,
    };
    rows.sort_by_key(|row| rank(row.status));
    Ok(ServicesOfMonth { pending_count: rows.iter().filter(|row| row.status == "pending").count(), rows, history_periods })
}

fn category_review(breakdown: &CategoryBreakdown, month: &str) -> Option<ReviewCard> {
    let row = breakdown.rows.iter().filter(|row| row.uncategorized).max_by_key(|row| cents(&row.amount))?;
    let share = row.percent.filter(|share| *share > 0.0).map_or_else(String::new, |share| format!(" — el {} % del mes", share.round()));
    Some(ReviewCard {
        id: format!("categories:{month}"),
        label: "Categorías".into(),
        parts: vec![
            strong(format!("{} {} sin categoría", breakdown.uncategorized_count, if breakdown.uncategorized_count == 1 { "gasto" } else { "gastos" })),
            text(format!(" {} {}{share}. Sin eso, el desglose no dice mucho.", if row.count == 1 { "suma" } else { "suman" }, spoken_money(cents(&row.amount), &row.currency))),
        ],
        actions: vec![ReviewAction { label: "Categorizar en el chat".into(), prompt: breakdown.categorize_prompt.clone().unwrap_or_default(), primary: true, option_id: None }],
        ask: breakdown.categorize_prompt.clone().unwrap_or_default(),
    })
}

pub fn finance_overview(app: crate::host::AppHandle, payload: MonthPayload) -> FinanceCommandResult<FinanceOverview> {
    valid_month(&payload.month)?;
    let connection = validate_context(&payload.context, &app)?;
    Ok(overview(&connection, payload.month)?)
}

fn overview(connection: &Connection, month: String) -> Result<FinanceOverview, String> {
    let reviews = pending_reviews(connection)?;
    let ledger = load_month(connection, &month, &reviews)?;
    let statements = load_statements_due(connection, &month)?;
    let statement_rows = statement_rows(&statements, &ledger.accounts);
    let mut card_totals = Totals::new();
    for statement in &statements {
        add(&mut card_totals, &statement.currency, statement.total_due);
    }
    let categories = category_breakdown(connection, &month, &ledger.entries)?;
    let mut review = reviews.iter().map(review_card).collect::<Vec<_>>();
    review.extend(category_review(&categories, &month));
    review.extend(statement_reviews(&statement_rows, &statements));
    let mut largest = ledger.entries.iter().filter(|entry| entry.counted_expense()).collect::<Vec<_>>();
    largest.sort_by(|left, right| left.row.currency.cmp(&right.row.currency).then(right.amount.cmp(&left.amount)));
    Ok(FinanceOverview {
        balance: month_balance(connection, &ledger.entries)?,
        budgets: crate::finance_budgets::budgets_of_month(connection, &month)?,
        loans: crate::finance_loans::loans_of_month(connection, &month)?,
        salary: salary_use(connection, &month)?,
        expenses: month_expenses(connection, &ledger.entries)?,
        saved: saved_this_month(connection, &month)?,
        largest: largest.into_iter().take(LARGEST_EXPENSES).map(|entry| entry.row.clone()).collect(),
        cards: CardsPaid { totals: money_list(&card_totals), statements: statement_rows, installments: installments_summary(connection)? },
        services: services_of_month(connection, &month, &reviews, &ledger.accounts)?,
        latest: ledger.entries.iter().take(LATEST_MOVEMENTS).map(|entry| entry.row.clone()).collect(),
        movement_count: ledger.entries.len(),
        categories,
        review,
        month,
    })
}

/// What the Home dashboard shows of Finanzas for a month: its expenses, the
/// share without a category, what was saved, the first reserve and how many
/// questions wait in «Para revisar».
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceHomeSummary {
    expenses: Vec<Money>,
    expense_count: usize,
    /// Whole percent of the month's expenses without a category.
    uncategorized_percent: Option<u8>,
    saved: Vec<Money>,
    reserve: Option<ReserveBalance>,
    review_count: usize,
    /// Every doubt of «Para revisar» as one request for the chat.
    review_prompt: Option<String>,
}

pub(crate) fn home_summary(
    app: &crate::host::AppHandle,
    context: &FinanceContext,
    month: &str,
) -> FinanceCommandResult<FinanceHomeSummary> {
    valid_month(month)?;
    let connection = validate_context(context, app)?;
    let overview = overview(&connection, month.to_string())?;
    let expense_count = overview.expenses.count;
    let uncategorized = overview.categories.uncategorized_count;
    Ok(FinanceHomeSummary {
        expenses: overview.expenses.totals,
        expense_count,
        uncategorized_percent: (expense_count > 0)
            .then(|| ((uncategorized.min(expense_count) * 100) as f64 / expense_count as f64).round() as u8),
        saved: overview.saved.contributions,
        reserve: overview.saved.reserves.into_iter().next(),
        review_count: overview.review.len(),
        review_prompt: review_request(&overview.review),
    })
}

// ---------------------------------------------------------------------------
// Movements
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementsPayload {
    context: FinanceContext,
    month: String,
    #[serde(default)]
    filter: String,
    #[serde(default)]
    search: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterChip {
    id: String,
    label: String,
    count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRef {
    total: Money,
    due_date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementGroup {
    account_id: String,
    badge: String,
    name: String,
    statement: Option<StatementRef>,
    kind_label: String,
    /// Counted expenses of the group.
    totals: Vec<Money>,
    /// What its currency exchanges put into savings.
    savings: Vec<Money>,
    rows: Vec<MovementRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovementsSummary {
    expense_count: usize,
    expense_totals: Vec<Money>,
    exchange_count: usize,
    income_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceMovements {
    month: String,
    filter: String,
    chips: Vec<FilterChip>,
    summary: MovementsSummary,
    groups: Vec<MovementGroup>,
    /// Prompt of «Pedir un cambio en el chat», per movement id.
    change_prompts: BTreeMap<String, String>,
}

fn chips(entries: &[Entry]) -> Vec<FilterChip> {
    let count = |filter: &str| entries.iter().filter(|entry| matches_filter(entry, filter)).count();
    let mut chips = vec![FilterChip { id: "all".into(), label: "Todos".into(), count: entries.len() }];
    chips.push(FilterChip { id: "uncategorized".into(), label: "Sin categoría".into(), count: count("uncategorized") });
    let mut categories = BTreeMap::<String, (String, i128, usize)>::new();
    for entry in entries.iter().filter(|entry| entry.counted_expense()) {
        if let (Some(id), Some(name)) = (&entry.category_id, &entry.row.category_name) {
            let category = categories.entry(id.clone()).or_insert_with(|| (name.clone(), 0, 0));
            category.1 += entry.amount;
            category.2 += 1;
        }
    }
    let mut categories = categories.into_iter().collect::<Vec<_>>();
    categories.sort_by(|left, right| right.1 .1.cmp(&left.1 .1));
    chips.extend(categories.into_iter().map(|(id, (label, _, count))| FilterChip { id: format!("category:{id}"), label, count }));
    for (id, label) in [("card-unpaid", "En tarjeta, a pagar"), ("pending", "Pendientes"), ("income", "Ingresos"), ("savings", "Ahorro"), ("discarded", "Descartados")] {
        chips.push(FilterChip { id: id.into(), label: label.into(), count: count(id) });
    }
    chips.retain(|chip| chip.id == "all" || chip.count > 0);
    chips
}

fn change_prompt(row: &MovementRow) -> String {
    format!(
        "Quiero cambiar este movimiento (id {}): «{}» del {} por {} {} en {}. ",
        row.id, row.description, row.purchase_date, row.currency, row.amount, row.account_name
    )
}

// ---------------------------------------------------------------------------
// The whole history: search and export
// ---------------------------------------------------------------------------

const SEARCH_PAGE: usize = 200;
const EVER_FROM: &str = "0000-01-01";
const EVER_TO: &str = "9999-12-31";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPayload {
    context: FinanceContext,
    #[serde(default)]
    search: String,
    #[serde(default)]
    filter: String,
    #[serde(default)]
    page: usize,
}

/// The movements found in one month.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMonth {
    month: String,
    rows: Vec<MovementRow>,
    /// The expenses found that count, per currency.
    expense_totals: Vec<Money>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSearch {
    search: String,
    filter: String,
    count: usize,
    page: usize,
    has_more: bool,
    months: Vec<SearchMonth>,
    expense_totals: Vec<Money>,
}

/// Movements of every month that match `search` and `filter` (the filters
/// of «Movimientos»), the most recent month first, by pages.
pub fn finance_search(app: crate::host::AppHandle, payload: SearchPayload) -> FinanceCommandResult<FinanceSearch> {
    let connection = validate_context(&payload.context, &app)?;
    Ok(search_history(&connection, &payload.search, &payload.filter, payload.page)?)
}

fn search_history(connection: &Connection, search: &str, filter: &str, page: usize) -> Result<FinanceSearch, String> {
    let reviews = pending_reviews(connection)?;
    let ledger = load_range(connection, EVER_FROM, EVER_TO, &reviews)?;
    let mut found = ledger.entries.iter().filter(|entry| matches_filter(entry, filter) && matches_search(entry, search)).collect::<Vec<_>>();
    found.sort_by(|left, right| right.row.effective_date.cmp(&left.row.effective_date).then(right.row.purchase_date.cmp(&left.row.purchase_date)));
    let mut totals = Totals::new();
    for entry in found.iter().filter(|entry| entry.counted_expense()) {
        add(&mut totals, &entry.row.currency, entry.amount);
    }
    let count = found.len();
    let start = page.saturating_mul(SEARCH_PAGE).min(count);
    let end = (start + SEARCH_PAGE).min(count);
    let mut months = Vec::<SearchMonth>::new();
    let mut month_totals = Vec::<Totals>::new();
    for entry in &found[start..end] {
        let month = entry.row.effective_date.get(..7).unwrap_or_default().to_string();
        if months.last().is_none_or(|last| last.month != month) {
            months.push(SearchMonth { month, rows: Vec::new(), expense_totals: Vec::new() });
            month_totals.push(Totals::new());
        }
        if entry.counted_expense() {
            if let Some(month_total) = month_totals.last_mut() {
                add(month_total, &entry.row.currency, entry.amount);
            }
        }
        if let Some(last) = months.last_mut() {
            last.rows.push(entry.row.clone());
        }
    }
    for (month, month_total) in months.iter_mut().zip(&month_totals) {
        month.expense_totals = money_list(month_total);
    }
    Ok(FinanceSearch {
        search: search.to_string(),
        filter: filter.to_string(),
        count,
        page,
        has_more: end < count,
        months,
        expense_totals: money_list(&totals),
    })
}

/// One movement as an exported row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExportRow {
    pub effective_date: String,
    pub purchase_date: String,
    pub kind: &'static str,
    pub description: String,
    pub category: String,
    pub account: String,
    pub service: String,
    pub amount: String,
    pub currency: String,
    pub status: &'static str,
    pub origin: String,
}

fn kind_text(kind: &str) -> &'static str {
    match kind {
        "expense" => "Gasto",
        "income" => "Ingreso",
        "transfer" => "Transferencia",
        "exchange" => "Cambio de moneda",
        _ => "Ajuste",
    }
}

fn status_text(status: &str) -> &'static str {
    match status {
        "confirmed" | "corrected" => "Cuenta",
        CARD_UNPAID => "En tarjeta, a pagar",
        "pending" => "Pendiente",
        "discarded" => "Descartado",
        _ => "Otro",
    }
}

/// The movements between two days (both in; empty for the whole history),
/// oldest first, as they are exported.
pub(crate) fn export_rows(connection: &Connection, from: Option<&str>, to: Option<&str>) -> Result<Vec<ExportRow>, String> {
    let ledger = load_range(connection, from.unwrap_or(EVER_FROM), to.unwrap_or(EVER_TO), &[])?;
    let mut rows = ledger
        .entries
        .iter()
        .map(|entry| ExportRow {
            effective_date: entry.row.effective_date.clone(),
            purchase_date: entry.row.purchase_date.clone(),
            kind: kind_text(&entry.row.kind),
            description: entry.row.description.clone(),
            category: entry.row.category_name.clone().unwrap_or_default(),
            account: entry.row.account_name.clone(),
            service: entry.row.service_name.clone().unwrap_or_default(),
            amount: entry.row.amount.clone(),
            currency: entry.row.currency.clone(),
            status: status_text(&entry.row.status),
            origin: entry.row.origin.clone(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.effective_date.cmp(&right.effective_date).then(left.purchase_date.cmp(&right.purchase_date)));
    Ok(rows)
}

pub fn finance_movements(app: crate::host::AppHandle, payload: MovementsPayload) -> FinanceCommandResult<FinanceMovements> {
    valid_month(&payload.month)?;
    let connection = validate_context(&payload.context, &app)?;
    Ok(movements(&connection, payload)?)
}

fn movements(connection: &Connection, payload: MovementsPayload) -> Result<FinanceMovements, String> {
    let reviews = pending_reviews(connection)?;
    let ledger = load_month(connection, &payload.month, &reviews)?;
    let statements = load_statements_due(connection, &payload.month)?;
    let chips = chips(&ledger.entries);
    let filter = if chips.iter().any(|chip| chip.id == payload.filter) { payload.filter } else { "all".to_string() };
    let shown = ledger
        .entries
        .iter()
        .filter(|entry| matches_filter(entry, &filter) && matches_search(entry, &payload.search))
        .collect::<Vec<_>>();

    let mut expense_totals = Totals::new();
    for entry in shown.iter().filter(|entry| entry.counted_expense()) {
        add(&mut expense_totals, &entry.row.currency, entry.amount);
    }
    let summary = MovementsSummary {
        expense_count: shown.iter().filter(|entry| entry.counted_expense()).count(),
        expense_totals: money_list(&expense_totals),
        exchange_count: shown.iter().filter(|entry| entry.row.kind == "exchange").count(),
        income_count: shown.iter().filter(|entry| entry.row.kind == "income" && counted(&entry.row.status)).count(),
    };

    let mut order = Vec::<String>::new();
    for statement in &statements {
        if !order.contains(&statement.account_id) {
            order.push(statement.account_id.clone());
        }
    }
    let mut others = shown.iter().map(|entry| entry.row.account_id.clone()).filter(|id| !order.contains(id)).collect::<Vec<_>>();
    others.sort_by_key(|id| {
        let account = ledger.accounts.get(id);
        (account.map_or(1, |account| u8::from(account.account_type != "credit_card")), account_label(&ledger.accounts, id).to_lowercase())
    });
    others.dedup();
    order.extend(others);

    let groups = order
        .into_iter()
        .filter_map(|account_id| {
            let rows = shown.iter().filter(|entry| entry.row.account_id == account_id).collect::<Vec<_>>();
            if rows.is_empty() {
                return None;
            }
            let (mut totals, mut savings) = (Totals::new(), Totals::new());
            for entry in &rows {
                if entry.counted_expense() {
                    add(&mut totals, &entry.row.currency, entry.amount);
                }
                if let Some(exchange) = entry.row.exchange.as_ref().filter(|_| counted(&entry.row.status)) {
                    add(&mut savings, &exchange.currency, cents(&exchange.amount));
                }
            }
            let account = ledger.accounts.get(&account_id);
            let statement = statements.iter().find(|statement| statement.account_id == account_id);
            Some(MovementGroup {
                badge: account.map_or_else(|| "—".into(), |account| account.badge.clone()),
                name: account_label(&ledger.accounts, &account_id),
                statement: statement.map(|statement| StatementRef { total: money(&statement.currency, statement.total_due), due_date: statement.due_date.clone() }),
                kind_label: account.map_or("Cuenta", |account| account_kind_label(&account.account_type)).to_string(),
                totals: money_list(&totals),
                savings: money_list(&savings),
                rows: rows.iter().map(|entry| entry.row.clone()).collect(),
                account_id,
            })
        })
        .collect::<Vec<_>>();
    let change_prompts = groups.iter().flat_map(|group| group.rows.iter()).map(|row| (row.id.clone(), change_prompt(row))).collect();
    Ok(FinanceMovements { month: payload.month, filter, chips, summary, groups, change_prompts })
}

// ---------------------------------------------------------------------------
// Products and tickets
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductsPayload {
    context: FinanceContext,
    #[serde(default)]
    search: String,
    /// `recent`, `rising` or `az`.
    #[serde(default)]
    sort: String,
    #[serde(default)]
    selected_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductRow {
    id: String,
    name: String,
    merchant_count: usize,
    last_date: String,
    /// Cheapest latest price among its merchants.
    from_price: Money,
    change_percent: Option<f64>,
    first_date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricePoint {
    date: String,
    price: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MerchantSeries {
    merchant: String,
    points: Vec<PricePoint>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MerchantPriceRow {
    merchant: String,
    price: String,
    date: String,
    cheapest: bool,
    /// Above the cheapest, in money and percent.
    difference: Option<String>,
    difference_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDetail {
    id: String,
    name: String,
    aliases: Vec<String>,
    currency: String,
    best: MerchantPriceRow,
    /// The most expensive latest price, when there is more than one merchant.
    worst: Option<MerchantPriceRow>,
    change_percent: Option<f64>,
    change_merchant: Option<String>,
    series: Vec<MerchantSeries>,
    rows: Vec<MerchantPriceRow>,
    similar: Option<ReviewCard>,
    correct_prompt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRow {
    id: String,
    date: String,
    merchant: String,
    item_count: i64,
    payment: String,
    /// `card-unpaid`, `paid-in-statement`, `confirmed`, `pending` or `discarded`.
    status: &'static str,
    total: Money,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceProducts {
    /// Products with a confirmed price, before the search.
    total_count: usize,
    products: Vec<ProductRow>,
    selected: Option<ProductDetail>,
    tickets: Vec<TicketRow>,
}

struct Observation {
    product_id: String,
    merchant: String,
    date: String,
    currency: String,
    price: i128,
}

/// Latest price and history of one product at each merchant, in the
/// product's most used currency, oldest first.
fn product_series<'a>(observations: &[&'a Observation]) -> (String, Vec<(String, Vec<&'a Observation>)>) {
    let mut by_currency = HashMap::<&str, usize>::new();
    for observation in observations {
        *by_currency.entry(observation.currency.as_str()).or_default() += 1;
    }
    let currency = by_currency.into_iter().max_by_key(|(currency, count)| (*count, *currency == "ARS")).map_or("ARS", |(currency, _)| currency).to_string();
    let mut merchants = Vec::<(String, Vec<&'a Observation>)>::new();
    for observation in observations.iter().copied().filter(|observation| observation.currency == currency) {
        match merchants.iter_mut().find(|(merchant, _)| *merchant == observation.merchant) {
            Some((_, list)) => list.push(observation),
            None => merchants.push((observation.merchant.clone(), vec![observation])),
        }
    }
    for (_, list) in &mut merchants {
        list.sort_by(|left, right| left.date.cmp(&right.date));
    }
    (currency, merchants)
}

/// Change since the first purchase at the merchant with most purchases.
fn price_change(merchants: &[(String, Vec<&Observation>)]) -> Option<(f64, String)> {
    let (merchant, list) = merchants.iter().max_by_key(|(_, list)| list.len())?;
    let (first, last) = (list.first()?.price, list.last()?.price);
    (list.len() > 1 && first > 0).then(|| (round1((last - first) as f64 / first as f64 * 100.0), merchant.clone()))
}

fn latest_prices(merchants: &[(String, Vec<&Observation>)]) -> Vec<MerchantPriceRow> {
    let latest = merchants.iter().filter_map(|(merchant, list)| list.last().map(|last| (merchant.clone(), last))).collect::<Vec<_>>();
    let best = latest.iter().map(|(_, observation)| observation.price).min().unwrap_or(0);
    let mut rows = latest
        .into_iter()
        .map(|(merchant, observation)| MerchantPriceRow {
            cheapest: observation.price == best,
            difference: (observation.price > best).then(|| format_cents(observation.price - best)),
            difference_percent: (observation.price > best && best > 0).then(|| round1((observation.price - best) as f64 / best as f64 * 100.0)),
            price: format_cents(observation.price),
            date: observation.date.get(..10).unwrap_or(&observation.date).to_string(),
            merchant,
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| cents(&row.price));
    rows
}

fn load_tickets(connection: &Connection, accounts: &HashMap<String, Account>) -> Result<Vec<TicketRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT p.id,substr(p.observed_at,1,10),COALESCE(m.name,'Sin comercio'),p.currency,p.total_amount,p.validation_status,
                    t.account_id,t.status,(SELECT COUNT(*) FROM finance_purchase_items i WHERE i.purchase_id=p.id),
                    EXISTS(SELECT 1 FROM finance_credit_card_statement_items c WHERE c.transaction_id=p.transaction_id)
             FROM finance_purchases p
             LEFT JOIN finance_merchants m ON m.id=p.merchant_id
             LEFT JOIN finance_transactions t ON t.id=p.transaction_id AND t.deleted_at IS NULL
             ORDER BY p.observed_at DESC,p.created_at DESC LIMIT ?1",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([TICKET_LIMIT as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, bool>(9)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .map(|(id, date, merchant, currency, total, validation, account_id, transaction_status, item_count, on_statement)| TicketRow {
            status: match (transaction_status.as_deref(), on_statement) {
                (Some(CARD_UNPAID), _) => "card-unpaid",
                (_, true) => "paid-in-statement",
                (Some("discarded"), _) => "discarded",
                _ if validation == "discarded" => "discarded",
                _ if validation == "pending" || transaction_status.as_deref() == Some("pending") => "pending",
                _ => "confirmed",
            },
            payment: account_id.as_deref().map_or_else(|| "—".to_string(), |id| account_label(accounts, id)),
            total: money(&currency, cents(&total)),
            id,
            date,
            merchant,
            item_count,
        })
        .collect())
}

pub fn finance_products(app: crate::host::AppHandle, payload: ProductsPayload) -> FinanceCommandResult<FinanceProducts> {
    let connection = validate_context(&payload.context, &app)?;
    Ok(products(&connection, payload)?)
}

fn products(connection: &Connection, payload: ProductsPayload) -> Result<FinanceProducts, String> {
    let products = connection
        .prepare("SELECT id,name FROM finance_products")
        .map_err(storage)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let observations = connection
        .prepare(
            "SELECT o.product_id,COALESCE(m.name,'Sin comercio'),o.observed_at,o.currency,o.unit_price
             FROM finance_price_observations o LEFT JOIN finance_merchants m ON m.id=o.merchant_id
             WHERE o.status IN ('confirmed','corrected')",
        )
        .map_err(storage)?
        .query_map([], |row| {
            Ok(Observation {
                product_id: row.get(0)?,
                merchant: row.get(1)?,
                date: row.get(2)?,
                currency: row.get(3)?,
                price: cents(&row.get::<_, String>(4)?),
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut by_product = HashMap::<&str, Vec<&Observation>>::new();
    for observation in &observations {
        by_product.entry(observation.product_id.as_str()).or_default().push(observation);
    }
    let mut aliases = HashMap::<String, Vec<String>>::new();
    let mut statement = connection
        .prepare("SELECT DISTINCT product_id,original_description FROM finance_purchase_items WHERE product_id IS NOT NULL")
        .map_err(storage)?;
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(storage)? {
        let (product, description) = row.map_err(storage)?;
        aliases.entry(product).or_default().push(description);
    }
    let search = normalize_text(&payload.search);
    let mut rows = Vec::new();
    let mut names = HashMap::new();
    for (id, name) in &products {
        let Some(list) = by_product.get(id.as_str()) else { continue };
        let (currency, merchants) = product_series(list);
        let latest = latest_prices(&merchants);
        let Some(best) = latest.first() else { continue };
        let product_aliases = aliases
            .get(id)
            .map(|list| {
                let mut seen = HashSet::from([normalize_text(name)]);
                list.iter().filter(|alias| seen.insert(normalize_text(alias))).take(5).cloned().collect::<Vec<_>>()
            })
            .unwrap_or_default();
        names.insert(id.clone(), (name.clone(), product_aliases.clone()));
        let haystack = normalize_text(&format!("{name} {}", product_aliases.join(" ")));
        if !search.split_whitespace().all(|word| haystack.contains(word)) {
            continue;
        }
        let dates = merchants.iter().flat_map(|(_, list)| list.iter().map(|observation| observation.date.as_str())).collect::<Vec<_>>();
        rows.push(ProductRow {
            id: id.clone(),
            name: name.clone(),
            merchant_count: merchants.len(),
            last_date: dates.iter().max().map_or("", |date| date.get(..10).unwrap_or(date)).to_string(),
            first_date: dates.iter().min().map_or("", |date| date.get(..10).unwrap_or(date)).to_string(),
            from_price: money(&currency, cents(&best.price)),
            change_percent: price_change(&merchants).map(|(change, _)| change),
        });
    }
    let total_count = names.len();
    match payload.sort.as_str() {
        "rising" => rows.sort_by(|left, right| right.change_percent.unwrap_or(f64::MIN).total_cmp(&left.change_percent.unwrap_or(f64::MIN))),
        "az" => rows.sort_by_key(|row| normalize_text(&row.name)),
        _ => rows.sort_by(|left, right| right.last_date.cmp(&left.last_date).then_with(|| left.name.cmp(&right.name))),
    }
    let selected_id = payload.selected_id.filter(|id| rows.iter().any(|row| &row.id == id)).or_else(|| rows.first().map(|row| row.id.clone()));
    let reviews = pending_reviews(connection)?;
    let selected = selected_id.and_then(|id| {
        let list = by_product.get(id.as_str())?;
        let (name, product_aliases) = names.get(&id)?.clone();
        let (currency, merchants) = product_series(list);
        let rows = latest_prices(&merchants);
        let best = rows.first()?;
        let worst = rows.last().filter(|_| rows.len() > 1);
        let change = price_change(&merchants);
        let similar = reviews
            .iter()
            .find(|review| review.kind == "similar-product" && (review.subject["newId"] == id.as_str() || review.subject["existingId"] == id.as_str()))
            .map(review_card);
        Some(ProductDetail {
            best: MerchantPriceRow { merchant: best.merchant.clone(), price: best.price.clone(), date: best.date.clone(), cheapest: true, difference: None, difference_percent: None },
            worst: worst.map(|worst| MerchantPriceRow {
                merchant: worst.merchant.clone(),
                price: worst.price.clone(),
                date: worst.date.clone(),
                cheapest: false,
                difference: worst.difference.clone(),
                difference_percent: worst.difference_percent,
            }),
            change_percent: change.as_ref().map(|(value, _)| *value),
            change_merchant: change.map(|(_, merchant)| merchant),
            series: merchants
                .iter()
                .map(|(merchant, list)| MerchantSeries {
                    merchant: merchant.clone(),
                    points: list.iter().map(|observation| PricePoint { date: observation.date.get(..10).unwrap_or(&observation.date).to_string(), price: format_cents(observation.price) }).collect(),
                })
                .collect(),
            correct_prompt: format!("Quiero corregir el producto «{name}» (id {id}): "),
            aliases: product_aliases,
            similar,
            rows,
            currency,
            name,
            id,
        })
    });
    let accounts = load_accounts(connection)?;
    Ok(FinanceProducts {
        total_count,
        products: rows,
        selected,
        tickets: load_tickets(connection, &accounts)?,
    })
}

// ---------------------------------------------------------------------------
// Salary and savings
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalaryBar {
    period: String,
    ars: f64,
    /// Converted with the official selling rate of the payment day.
    usd: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestSalary {
    period: String,
    net: String,
    currency: String,
    payment_date: Option<String>,
    previous_period: Option<String>,
    change_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalaryChart {
    employers: Vec<String>,
    /// Up to twelve months, the last one being the salary that pays the month.
    bars: Vec<SalaryBar>,
    average_ars: Option<f64>,
    average_usd: Option<f64>,
    latest: Option<LatestSalary>,
    dollar_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InflationComparison {
    from: String,
    to: String,
    ars_change: f64,
    usd_change: f64,
    ipc: f64,
    ars_vs_ipc: f64,
    usd_vs_ipc: f64,
}

/// A part of the salary month by month: cards, services apart from the card
/// and savings, over the salary that paid them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalaryShare {
    key: &'static str,
    current: Option<f64>,
    amount: Option<Money>,
    salary: Option<Money>,
    periods: Vec<String>,
    values: Vec<Option<f64>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteCard {
    kind: String,
    name: String,
    buy: f64,
    sell: f64,
    spread: f64,
    updated_at: String,
    /// Selling price over the official one, in percent.
    gap_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastDollarPurchase {
    date: String,
    rate: String,
    /// Rate paid minus today's official and blue selling prices.
    vs_official: Option<f64>,
    vs_blue: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveMovementRow {
    id: String,
    date: String,
    reason: String,
    movement_type: String,
    amount: String,
    currency: String,
    cost: Option<Money>,
    rate: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindAmount {
    kind: String,
    amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveView {
    id: String,
    name: String,
    currency: String,
    balance: String,
    month_change: String,
    /// What the balance is worth in pesos at today's buying prices, by quote.
    valuation: Vec<KindAmount>,
    /// The month's movements by type.
    totals: Vec<KindAmount>,
    movements: Vec<ReserveMovementRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSalarySavings {
    month: String,
    salary: SalaryChart,
    inflation: Option<InflationComparison>,
    inflation_error: Option<String>,
    shares: Vec<SalaryShare>,
    quotes: Vec<QuoteCard>,
    quotes_error: Option<String>,
    last_purchase: Option<LastDollarPurchase>,
    reserves: Vec<ReserveView>,
}

struct SalaryRecords {
    receipts: Vec<(String, String, f64, String, String)>,
    shares: Vec<SalaryShare>,
    reserves: Vec<(crate::finance::FinanceSavingsReserve, Vec<ReserveMovementRow>)>,
    last_purchase: Option<(String, String)>,
}

fn salary_shares(connection: &Connection, month: &str) -> Result<Vec<SalaryShare>, String> {
    let periods = (0..SHARE_MONTHS).rev().filter_map(|offset| shift_month(month, -offset)).collect::<Vec<_>>();
    let currency = previous_period(month).and_then(|period| salary_of(connection, &period).ok().flatten()).map_or("ARS".to_string(), |salary| salary.currency);
    let mut columns = Vec::new();
    for period in &periods {
        let salary = previous_period(period).and_then(|previous| salary_of(connection, &previous).ok().flatten()).filter(|salary| salary.currency == currency);
        columns.push((month_flows(connection, period, &currency)?, salary.map(|salary| salary.net)));
    }
    let share = |key: &'static str, pick: fn(&Flows) -> (i128, bool)| {
        let values = columns
            .iter()
            .map(|(flows, salary)| {
                let (amount, recorded) = pick(flows);
                if recorded { salary.and_then(|salary| percent(amount, salary)) } else { None }
            })
            .collect::<Vec<_>>();
        let (flows, salary) = columns.last().copied().unwrap_or_default();
        SalaryShare {
            key,
            current: values.last().copied().flatten(),
            amount: pick(&flows).1.then(|| money(&currency, pick(&flows).0)),
            salary: salary.map(|salary| money(&currency, salary)),
            periods: periods.clone(),
            values,
        }
    };
    Ok(vec![share("cards", |flows| flows.cards), share("services", |flows| flows.services_apart), share("savings", |flows| flows.savings)])
}

fn reserve_movements(connection: &Connection, month: &str) -> Result<Vec<(crate::finance::FinanceSavingsReserve, Vec<ReserveMovementRow>)>, String> {
    let reserves = crate::finance::finance_list_savings_inner(connection)?.into_iter().filter(|reserve| reserve.active).collect::<Vec<_>>();
    let mut statement = connection
        .prepare(
            "SELECT m.id,m.reserve_id,substr(m.effective_date,1,10),COALESCE(NULLIF(m.reason,''),m.description),m.movement_type,m.amount,m.currency,
                    t.currency,t.amount
             FROM finance_savings_movements m
             LEFT JOIN finance_transactions t ON t.id=m.linked_transaction_id AND t.transaction_type='exchange' AND t.deleted_at IS NULL
             WHERE m.status IN ('confirmed','corrected') AND substr(m.effective_date,1,7)=?1
             ORDER BY m.effective_date DESC,m.created_at DESC",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([month], |row| {
            Ok((
                row.get::<_, String>(1)?,
                ReserveMovementRow {
                    id: row.get(0)?,
                    date: row.get(2)?,
                    reason: row.get(3)?,
                    movement_type: row.get(4)?,
                    amount: format_cents(cents(&row.get::<_, String>(5)?)),
                    currency: row.get(6)?,
                    cost: row.get::<_, Option<String>>(7)?.zip(row.get::<_, Option<String>>(8)?).map(|(currency, amount)| money(&currency, cents(&amount))),
                    rate: None,
                },
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(reserves
        .into_iter()
        .map(|reserve| {
            let movements = rows
                .iter()
                .filter(|(reserve_id, _)| *reserve_id == reserve.id)
                .map(|(_, movement)| ReserveMovementRow {
                    rate: movement.cost.as_ref().filter(|cost| cost.currency != movement.currency).and_then(|cost| rate(cents(&cost.amount), cents(&movement.amount))),
                    id: movement.id.clone(),
                    date: movement.date.clone(),
                    reason: movement.reason.clone(),
                    movement_type: movement.movement_type.clone(),
                    amount: movement.amount.clone(),
                    currency: movement.currency.clone(),
                    cost: movement.cost.clone(),
                })
                .collect();
            (reserve, movements)
        })
        .collect())
}

/// Latest dollar bought for savings up to the end of the month: its day and
/// the rate paid.
fn last_dollar_purchase(connection: &Connection, month: &str) -> Result<Option<(String, String)>, String> {
    let (_, to) = notia_backend_core::finance_insights::month_range(month).ok_or("El mes no es válido.")?;
    let row = connection
        .prepare(
            "SELECT substr(t.effective_date,1,10),t.amount,m.amount FROM finance_transactions t
             JOIN finance_savings_movements m ON m.linked_transaction_id=t.id AND m.movement_type='contribution'
             WHERE t.transaction_type='exchange' AND t.deleted_at IS NULL AND t.status IN ('confirmed','corrected')
               AND t.currency='ARS' AND m.currency='USD' AND substr(t.effective_date,1,10)<=?1
             ORDER BY t.effective_date DESC,t.created_at DESC LIMIT 1",
        )
        .map_err(storage)?
        .query_map([to], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(storage)?
        .next()
        .transpose()
        .map_err(storage)?;
    Ok(row.and_then(|(date, paid, bought)| rate(cents(&paid), cents(&bought)).map(|rate| (date, rate))))
}

fn read_salary_records(connection: &Connection, month: &str) -> Result<SalaryRecords, String> {
    let last_period = previous_period(month).unwrap_or_default();
    let receipts = connection
        .prepare(
            "SELECT period,COALESCE(payment_date,period || '-01'),net_amount,currency,COALESCE(employer,'')
             FROM finance_salary_receipts WHERE period<=?1 ORDER BY period",
        )
        .map_err(storage)?
        .query_map([&last_period], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, cents(&row.get::<_, String>(2)?) as f64 / 100.0, row.get::<_, String>(3)?, row.get::<_, String>(4)?))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(SalaryRecords {
        receipts,
        shares: salary_shares(connection, month)?,
        reserves: reserve_movements(connection, month)?,
        last_purchase: last_dollar_purchase(connection, month)?,
    })
}

fn salary_chart(receipts: &[(String, String, f64, String, String)], points: Option<&[notia_backend_core::finance_insights::SalaryPoint]>, dollar_error: Option<String>) -> SalaryChart {
    let mut periods = BTreeMap::<String, (f64, Option<f64>)>::new();
    match points {
        Some(points) => {
            for point in points {
                periods.insert(point.period.clone(), (point.ars, Some(point.usd)));
            }
        }
        None => {
            for (period, _, net, _, _) in receipts.iter().filter(|receipt| receipt.3 == "ARS") {
                periods.entry(period.clone()).or_insert((0.0, None)).0 += net;
            }
        }
    }
    let bars = periods
        .into_iter()
        .rev()
        .take(SALARY_BARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(period, (ars, usd))| SalaryBar { period, ars, usd })
        .collect::<Vec<_>>();
    let average = |values: Vec<f64>| (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
    let latest = bars.last().map(|last| {
        let previous = bars.len().checked_sub(2).and_then(|index| bars.get(index));
        LatestSalary {
            net: format!("{:.2}", last.ars),
            currency: "ARS".into(),
            payment_date: receipts.iter().filter(|receipt| receipt.0 == last.period).map(|receipt| receipt.1.clone()).max(),
            previous_period: previous.map(|previous| previous.period.clone()),
            change_percent: previous.filter(|previous| previous.ars > 0.0).map(|previous| round1((last.ars - previous.ars) / previous.ars * 100.0)),
            period: last.period.clone(),
        }
    });
    let mut employers = Vec::new();
    for (period, _, _, _, employer) in receipts.iter().rev() {
        if bars.iter().any(|bar| &bar.period == period) && !employer.trim().is_empty() && !employers.contains(employer) {
            employers.push(employer.clone());
        }
    }
    SalaryChart {
        average_ars: average(bars.iter().map(|bar| bar.ars).collect()),
        average_usd: average(bars.iter().filter_map(|bar| bar.usd).collect()).filter(|_| bars.iter().all(|bar| bar.usd.is_some())),
        employers,
        latest,
        dollar_error,
        bars,
    }
}

fn quote_cards(quotes: &[crate::services::finance_external::DollarQuote]) -> Vec<QuoteCard> {
    let official = quotes.iter().find(|quote| quote.kind == "oficial").map(|quote| quote.sell).filter(|sell| *sell > 0.0);
    quotes
        .iter()
        .map(|quote| QuoteCard {
            gap_percent: official.filter(|_| quote.kind != "oficial").map(|official| round1((quote.sell / official - 1.0) * 100.0)),
            kind: quote.kind.clone(),
            name: quote.name.clone(),
            buy: quote.buy,
            sell: quote.sell,
            spread: quote.sell - quote.buy,
            updated_at: quote.updated_at.clone(),
        })
        .collect()
}

pub async fn finance_salary_savings(app: crate::host::AppHandle, payload: MonthPayload) -> FinanceCommandResult<FinanceSalarySavings> {
    use notia_backend_core::finance_insights as insights;
    valid_month(&payload.month)?;
    let month = payload.month.clone();
    let context = payload.context;
    let records = crate::host::async_runtime::spawn_blocking(move || -> Result<SalaryRecords, String> {
        let connection = validate_context(&context, &app)?;
        read_salary_records(&connection, &month)
    })
    .await
    .map_err(|_| "No se pudieron leer el sueldo y el ahorro.".to_string())??;

    let (quotes, quotes_error) = match crate::services::finance_external::dollar_quotes().await {
        Ok(quotes) => (quotes, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let quote = |kind: &str| quotes.iter().find(|quote| quote.kind == kind);

    let (points, dollar_error) = if records.receipts.is_empty() {
        (None, None)
    } else {
        match crate::services::finance_external::historical_dollar_quotes(None, None).await {
            Ok(history) => {
                let mut history = history.into_iter().map(|quote| (quote.date, quote.sell)).collect::<Vec<_>>();
                history.sort_by(|left, right| left.0.cmp(&right.0));
                let inputs = records
                    .receipts
                    .iter()
                    .map(|(period, payment_date, net, currency, _)| insights::SalaryInput { period: period.clone(), payment_date: payment_date.clone(), net: *net, currency: currency.clone() })
                    .collect::<Vec<_>>();
                (Some(insights::salary_points(&inputs, &history)), None)
            }
            Err(error) => (None, Some(error)),
        }
    };
    let salary = salary_chart(&records.receipts, points.as_deref(), dollar_error);

    let summary = points.as_deref().zip(salary.latest.as_ref()).and_then(|(points, latest)| insights::year_summary_at(points, &latest.period));
    let (inflation, inflation_error) = match summary {
        Some(summary) => match crate::services::finance_external::finance_inflation_indices().await {
            Ok(indices) => {
                let monthly = indices.monthly.into_iter().map(|index| (index.period, index.percent)).collect::<BTreeMap<_, _>>();
                let annual = indices.annual.into_iter().map(|index| (index.period, index.percent)).collect::<BTreeMap<_, _>>();
                let benchmark = insights::inflation_benchmark(&summary, &monthly, &annual);
                (
                    benchmark.map(|benchmark| InflationComparison {
                        from: summary.comparison_period.clone(),
                        to: summary.current_period.clone(),
                        ars_change: summary.ars_variation_percent,
                        usd_change: summary.usd_variation_percent,
                        ipc: benchmark.ipc_accumulated_percent,
                        ars_vs_ipc: benchmark.ars_vs_ipc_percentage_points,
                        usd_vs_ipc: benchmark.usd_vs_ipc_percentage_points,
                    }),
                    None,
                )
            }
            Err(error) => (None, Some(error)),
        },
        None => (None, None),
    };

    let last_purchase = records.last_purchase.map(|(date, paid)| {
        let paid_value = cents(&paid) as f64 / 100.0;
        LastDollarPurchase {
            vs_official: quote("oficial").map(|quote| (paid_value - quote.sell).round()),
            vs_blue: quote("blue").map(|quote| (paid_value - quote.sell).round()),
            rate: paid,
            date,
        }
    });

    let reserves = records
        .reserves
        .into_iter()
        .map(|(reserve, movements)| {
            let mut totals = BTreeMap::<String, i128>::new();
            for movement in &movements {
                *totals.entry(movement.movement_type.clone()).or_default() += cents(&movement.amount);
            }
            let value = |key: &str| totals.get(key).copied().unwrap_or(0);
            let change = value("contribution") + value("return") + value("adjustment") - value("withdrawal") - value("loss");
            let balance = cents(&reserve.balance);
            let valuation = if reserve.currency == "USD" {
                ["oficial", "blue"]
                    .into_iter()
                    .filter_map(|kind| quote(kind).map(|quote| KindAmount { kind: kind.to_string(), amount: format!("{:.2}", balance as f64 / 100.0 * quote.buy) }))
                    .collect()
            } else {
                Vec::new()
            };
            ReserveView {
                totals: totals.into_iter().filter(|(_, amount)| *amount != 0).map(|(kind, amount)| KindAmount { kind, amount: format_cents(amount) }).collect(),
                month_change: format_cents(change),
                balance: format_cents(balance),
                id: reserve.id,
                name: reserve.name,
                currency: reserve.currency,
                valuation,
                movements,
            }
        })
        .collect();

    Ok(FinanceSalarySavings {
        month: payload.month,
        quotes: quote_cards(&quotes),
        shares: records.shares,
        salary,
        inflation,
        inflation_error,
        quotes_error,
        last_purchase,
        reserves,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dumps the four tabs of a real library for the visual preview of the
    /// screen: `FIN_LIBRARY` (a folder with `.notia/notia.db`, a copy),
    /// `FIN_OUT` and `FIN_MONTH` (`AAAA-MM`). The salary tab asks the
    /// quote services, so it needs the network.
    #[test]
    #[ignore]
    fn dump_preview_fixtures() {
        let (Ok(library), Ok(out), Ok(month)) = (std::env::var("FIN_LIBRARY"), std::env::var("FIN_OUT"), std::env::var("FIN_MONTH")) else { return };
        let app = crate::create_app(crate::host::AppPaths::new(Some(std::path::Path::new(&out).join("app")), None), crate::host::HostPorts::default());
        let context = serde_json::json!({ "libraryPath": library, "androidDirectoryUri": null, "actorLibraryUserId": "user-owner", "source": "app" });
        let write = |name: &str, value: serde_json::Value| std::fs::write(std::path::Path::new(&out).join(name), value.to_string()).expect("write");
        let month_payload = || serde_json::from_value::<MonthPayload>(serde_json::json!({ "context": context, "month": month })).expect("month");
        write("overview.json", serde_json::to_value(finance_overview(app.clone(), month_payload()).expect("overview")).expect("json"));
        let movements = serde_json::from_value::<MovementsPayload>(serde_json::json!({ "context": context, "month": month })).expect("movements");
        write("movements.json", serde_json::to_value(finance_movements(app.clone(), movements).expect("movements")).expect("json"));
        let products = serde_json::from_value::<ProductsPayload>(serde_json::json!({ "context": context })).expect("products");
        let list = finance_products(app.clone(), products).expect("products");
        write("products.json", serde_json::to_value(&list).expect("json"));
        let salary = crate::host::async_runtime::block_on(finance_salary_savings(app.clone(), month_payload())).expect("salary");
        write("salary.json", serde_json::to_value(salary).expect("json"));
    }

    fn seeded() -> Connection {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        crate::finance::seed_finance_demo_data(&mut connection).expect("demo seed");
        connection
    }

    fn movements_payload(month: &str, filter: &str, search: &str) -> MovementsPayload {
        serde_json::from_value(serde_json::json!({
            "context": { "libraryPath": "", "androidDirectoryUri": null, "actorLibraryUserId": "user-owner", "source": "app" },
            "month": month, "filter": filter, "search": search
        }))
        .expect("payload")
    }

    #[test]
    fn overview_derives_the_salary_use_cards_and_reviews_of_the_month() {
        let connection = seeded();
        let overview = overview(&connection, "2026-09".into()).expect("overview");
        let salary = overview.salary.expect("salary of august");
        assert_eq!((salary.period.as_str(), salary.net.as_str()), ("2026-08", "128000.00"));
        let segment = |key: &str| salary.segments.iter().find(|segment| segment.key == key).expect("segment");
        assert_eq!((segment("cards").amount.as_str(), segment("cards").percent), ("15250.00", Some(11.9)));
        assert_eq!(segment("unregistered").amount, "112750.00");
        assert_eq!(overview.cards.statements[0].check, "matches");
        assert_eq!(overview.cards.statements[0].name, "Banco Demo ••1234");
        assert_eq!(overview.cards.installments.pending_count, 2);
        assert_eq!((overview.expenses.count, overview.expenses.card_unpaid_count), (1, 1));
        assert!(overview.review.iter().any(|card| card.label == "Producto" && card.actions.len() == 2));
        assert_eq!(overview.services.pending_count, 2);
        assert_eq!(overview.latest.len(), 2);
    }

    #[test]
    fn home_asks_about_every_doubt_without_answering_it() {
        let card = |id: &str, ask: &str| ReviewCard { id: id.into(), label: String::new(), parts: Vec::new(), actions: Vec::new(), ask: ask.into() };
        assert_eq!(review_request(&[]), None);
        let categories = card("categories:2026-09", "Ayudame a categorizar los 9 gastos sin categoría de septiembre.");
        assert_eq!(review_request(std::slice::from_ref(&categories)).as_deref(), Some("Ayudame a categorizar los 9 gastos sin categoría de septiembre."));
        let doubt = card("r1", "Revisemos la duda de Finanzas «¿Es el mismo comercio?» (caso r1).");
        assert_eq!(
            review_request(&[doubt, categories]).as_deref(),
            Some("Revisemos lo que quedó para revisar en Finanzas:\n- Revisemos la duda de Finanzas «¿Es el mismo comercio?» (caso r1).\n- Ayudame a categorizar los 9 gastos sin categoría de septiembre.")
        );

        let connection = seeded();
        let overview = overview(&connection, "2026-09".into()).expect("overview");
        let request = review_request(&overview.review).expect("request");
        let product = overview.review.iter().find(|card| card.label == "Producto").expect("product doubt");
        assert!(request.contains(&format!("(caso {})", product.id)));
        assert!(!request.contains(&product.actions[0].label), "the request does not pick an answer");
    }

    #[test]
    fn august_shows_what_the_dollars_for_savings_cost() {
        let connection = seeded();
        let overview = overview(&connection, "2026-08".into()).expect("overview");
        assert_eq!(overview.saved.rate.as_deref(), Some("1450.00"));
        assert_eq!(overview.saved.bought, vec![money("USD", 10_000)]);
        let exchange = overview.latest.iter().find(|row| row.kind == "exchange").and_then(|row| row.exchange.as_ref()).expect("exchange");
        assert_eq!((exchange.amount.as_str(), exchange.rate.as_deref(), exchange.into_savings), ("100.00", Some("1450.00"), true));
        let july_statement = &overview.cards.statements[0];
        assert_eq!(july_statement.check, "matches");
    }

    #[test]
    fn movements_filter_search_and_group_by_account() {
        let connection = seeded();
        let all = movements(&connection, movements_payload("2026-08", "all", "")).expect("movements");
        assert!(all.chips.iter().any(|chip| chip.id == "discarded" && chip.count == 1));
        assert!(all.chips.iter().any(|chip| chip.id == "savings" && chip.count == 2));
        assert_eq!(all.groups[0].account_id, "dev-account-card");
        let discarded = movements(&connection, movements_payload("2026-08", "discarded", "")).expect("discarded");
        let row = &discarded.groups[0].rows[0];
        assert_eq!(row.status, "discarded");
        assert!(row.note.as_deref().is_some_and(|note| note.starts_with("Descartado")));
        let found = movements(&connection, movements_payload("2026-08", "all", "supermercado")).expect("search");
        assert_eq!(found.summary.expense_count, 1);
        let unknown = movements(&connection, movements_payload("2026-08", "category:none", "")).expect("unknown filter");
        assert_eq!(unknown.filter, "all");
    }

    #[test]
    fn products_keep_confirmed_prices_their_change_and_similar_doubt() {
        let connection = seeded();
        let payload = serde_json::from_value::<ProductsPayload>(serde_json::json!({
            "context": { "libraryPath": "", "androidDirectoryUri": null, "actorLibraryUserId": "user-owner", "source": "app" },
            "search": "", "sort": "recent"
        }))
        .expect("payload");
        let products = products(&connection, payload).expect("products");
        assert_eq!(products.total_count, 1);
        assert_eq!(products.products[0].change_percent, Some(15.6));
        let detail = products.selected.expect("milk selected");
        assert_eq!(detail.best.price, "5200.00");
        assert!(detail.similar.is_some());
        assert_eq!(products.tickets.len(), 2);
    }

    #[test]
    fn salary_shares_are_over_the_salary_that_paid_each_month() {
        let connection = seeded();
        let shares = salary_shares(&connection, "2026-09").expect("shares");
        let cards = shares.iter().find(|share| share.key == "cards").expect("cards");
        assert_eq!(cards.current, Some(11.9));
        assert_eq!(cards.values[10], Some(10.4));
        let savings = shares.iter().find(|share| share.key == "savings").expect("savings");
        assert_eq!(savings.current, None);
        assert_eq!(savings.values[10], Some(145.8));
    }

    #[test]
    fn wording_helpers() {
        assert_eq!(spoken_money(49_686_009, "ARS"), "$ 496.860");
        assert_eq!(spoken_money(100_000, "USD"), "USD 1.000");
        assert_eq!(month_label("2026-09"), "septiembre de 2026");
        assert_eq!(short_date("2026-08-05"), "5 ago");
        assert_eq!(percent(66_913_999, 525_043_263), Some(12.7));
        assert_eq!(rate(153_000_000, 100_000), Some("1530.00".into()));
    }

    #[test]
    fn questions_bold_what_they_are_about() {
        let parts = question_parts("¿De qué servicio es el consumo «AMP2008 10/18» del 2026-08-05?");
        assert_eq!(parts.len(), 3);
        assert!(parts[1].strong);
        assert_eq!(parts[1].text, "AMP2008 10/18");
        assert_eq!(question_parts("Sin comillas")[0].text, "Sin comillas");
    }

    #[test]
    fn statement_lines_are_checked_against_the_new_part_of_the_total() {
        let line = |item_type: &str, amount: i128| StatementLine { item_type: item_type.into(), amount, description: String::new(), transaction_status: None };
        let statement = Statement {
            id: "s".into(),
            account_id: "a".into(),
            due_date: "2026-09-04".into(),
            currency: "ARS".into(),
            previous_balance: 50_000,
            payments: 50_000,
            total_due: 30_000,
            lines: vec![line("purchase", 25_000), line("tax", 6_000), line("credit", 1_000), line("payment", 50_000)],
        };
        assert_eq!(lines_total(&statement), 30_000);
        assert_eq!(expected_lines(&statement), 30_000);
        assert_eq!(card_badge("Mastercard Galicia"), "MC");
    }
}
