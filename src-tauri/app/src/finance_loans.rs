//! Préstamos: lo que se debe y sus cuotas.
//!
//! Un préstamo tiene un calendario mensual de cuotas desde el primer
//! vencimiento. Pagar una cuota crea un gasto de la categoría «Préstamos»
//! (cuenta en el mes en que se pagó) y lo une a la cuota; el importe pagado
//! puede diferir del esperado (por ejemplo, en un préstamo UVA). Desmarcar
//! la cuota borra ese gasto. El gasto pertenece a la cuota: desde
//! Movimientos solo cambian su categoría y su descripción.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::finance::{
    format_cents, now, parse_cents, sync_context, valid_iso_date, validate_context, FinanceCommandError,
    FinanceCommandResult, FinanceContext, FinanceTransaction,
};
use crate::finance_matching::add_months;

/// Category of the installments paid, created the first time one is paid.
pub(crate) const LOANS_CATEGORY: &str = "category-loans";
const LOANS_CATEGORY_NAME: &str = "Préstamos";
const MAX_INSTALLMENTS: u32 = 600;
const MAX_NAME_CHARS: usize = 120;
const MAX_NOTES_CHARS: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Loan {
    pub id: String,
    pub name: String,
    pub lender: Option<String>,
    pub currency: String,
    /// What was received, when the person says it.
    pub principal: Option<String>,
    pub installment_count: u32,
    pub installment_amount: String,
    pub first_due_date: String,
    /// The account it is usually paid from.
    pub account_id: Option<String>,
    pub notes: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoanInstallment {
    pub number: u32,
    pub due_date: String,
    pub expected_amount: String,
    pub paid_amount: Option<String>,
    pub paid_date: Option<String>,
    pub transaction_id: Option<String>,
}

/// A loan with its schedule and what is left.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanSummary {
    #[serde(flatten)]
    pub loan: Loan,
    pub installments: Vec<LoanInstallment>,
    pub paid_count: u32,
    /// Expected amount of the installments still unpaid.
    pub remaining: String,
    pub paid_total: String,
    pub next_due: Option<LoanInstallment>,
}

/// What paying an installment says.
#[derive(Debug, Clone)]
pub(crate) struct Payment {
    pub number: Option<u32>,
    pub amount: Option<String>,
    pub date: String,
    pub account_id: Option<String>,
}

fn storage(error: rusqlite::Error) -> String {
    format!("No se pudieron leer los préstamos: {error}")
}

fn valid_money(value: &str) -> bool {
    parse_cents(value).is_ok_and(|cents| cents > 0)
}

/// What is wrong with `loan`, naming the field.
pub(crate) fn loan_problem(loan: &Loan) -> Option<(&'static str, String)> {
    if loan.id.trim().is_empty() {
        return Some(("id", "Falta el identificador del préstamo.".into()));
    }
    let name = loan.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Some(("name", format!("Escribí un nombre de hasta {MAX_NAME_CHARS} letras.")));
    }
    if !matches!(loan.currency.as_str(), "ARS" | "USD") {
        return Some(("currency", "Elegí pesos o dólares.".into()));
    }
    if loan.installment_count == 0 || loan.installment_count > MAX_INSTALLMENTS {
        return Some(("installmentCount", format!("Las cuotas van de 1 a {MAX_INSTALLMENTS}.")));
    }
    if !valid_money(&loan.installment_amount) {
        return Some(("installmentAmount", "Escribí el importe de cada cuota, mayor que cero.".into()));
    }
    if loan.principal.as_deref().is_some_and(|value| !valid_money(value)) {
        return Some(("principal", "Escribí lo que recibiste, mayor que cero, o dejalo vacío.".into()));
    }
    if !valid_iso_date(&loan.first_due_date) {
        return Some(("firstDueDate", "Elegí la fecha del primer vencimiento.".into()));
    }
    if loan.notes.as_deref().is_some_and(|notes| notes.chars().count() > MAX_NOTES_CHARS) {
        return Some(("notes", format!("Las notas van hasta {MAX_NOTES_CHARS} letras.")));
    }
    None
}

/// Saves a loan and its schedule. Paid installments keep their payment;
/// the unpaid ones follow the new amount and dates, and the ones beyond the
/// new count go away.
pub(crate) fn save_loan(app: &crate::host::AppHandle, context: &FinanceContext, loan: &Loan) -> FinanceCommandResult<LoanSummary> {
    if let Some((_, message)) = loan_problem(loan) {
        return Err(FinanceCommandError::invalid(message));
    }
    let connection = validate_context(context, app)?;
    if let Some(account_id) = loan.account_id.as_deref().filter(|id| !id.is_empty()) {
        let currency: Option<(String, String)> = connection
            .query_row(
                "SELECT currency,account_type FROM finance_accounts WHERE id=?1 AND active=1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        match currency {
            None => return Err(FinanceCommandError::invalid("La cuenta del préstamo no existe o está inactiva.")),
            Some((currency, kind)) if kind != "credit_card" && currency != loan.currency => {
                return Err(FinanceCommandError::invalid("La cuenta del préstamo tiene que ser de la misma moneda."));
            }
            Some(_) => {}
        }
    }
    let paid_beyond: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM finance_loan_installments WHERE loan_id=?1 AND number>?2 AND transaction_id IS NOT NULL)",
            params![loan.id, loan.installment_count],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if paid_beyond {
        return Err(FinanceCommandError::invalid("Hay cuotas pagadas después de esa cantidad: desmarcalas antes de achicar el préstamo."));
    }
    let timestamp = now();
    let transaction = connection.unchecked_transaction().map_err(storage)?;
    transaction
        .execute(
            "INSERT INTO finance_loans (id,name,lender,currency,principal,installment_count,installment_amount,first_due_date,account_id,notes,active,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,lender=excluded.lender,currency=excluded.currency,principal=excluded.principal,
               installment_count=excluded.installment_count,installment_amount=excluded.installment_amount,first_due_date=excluded.first_due_date,
               account_id=excluded.account_id,notes=excluded.notes,active=excluded.active,deleted_at=NULL,updated_at=excluded.updated_at",
            params![
                loan.id,
                loan.name.trim(),
                loan.lender.as_deref().map(str::trim).filter(|value| !value.is_empty()),
                loan.currency,
                loan.principal.as_deref().map(|value| format_cents(parse_cents(value).unwrap_or_default())),
                loan.installment_count,
                format_cents(parse_cents(&loan.installment_amount).unwrap_or_default()),
                loan.first_due_date,
                loan.account_id.as_deref().filter(|id| !id.is_empty()),
                loan.notes.as_deref().map(str::trim).filter(|value| !value.is_empty()),
                loan.active,
                timestamp,
            ],
        )
        .map_err(storage)?;
    let amount = format_cents(parse_cents(&loan.installment_amount).unwrap_or_default());
    for number in 1..=loan.installment_count {
        let offset = i32::try_from(number - 1).map_err(|_| "Demasiadas cuotas.".to_string())?;
        let due_date = add_months(&loan.first_due_date, offset)?;
        transaction
            .execute(
                "INSERT INTO finance_loan_installments (id,loan_id,number,due_date,expected_amount,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?6)
                 ON CONFLICT(loan_id,number) DO UPDATE SET
                   due_date=CASE WHEN finance_loan_installments.transaction_id IS NULL THEN excluded.due_date ELSE finance_loan_installments.due_date END,
                   expected_amount=CASE WHEN finance_loan_installments.transaction_id IS NULL THEN excluded.expected_amount ELSE finance_loan_installments.expected_amount END,
                   updated_at=excluded.updated_at",
                params![format!("{}:{number}", loan.id), loan.id, number, due_date, amount, timestamp],
            )
            .map_err(storage)?;
    }
    transaction
        .execute(
            "DELETE FROM finance_loan_installments WHERE loan_id=?1 AND number>?2 AND transaction_id IS NULL",
            params![loan.id, loan.installment_count],
        )
        .map_err(storage)?;
    transaction.commit().map_err(storage)?;
    let summary = load_loan(&connection, &loan.id)?.ok_or("El préstamo no se guardó.")?;
    drop(connection);
    sync_context(context, app)?;
    Ok(summary)
}

/// Deletes a loan and the expenses that paid its installments.
pub(crate) fn delete_loan(app: &crate::host::AppHandle, context: &FinanceContext, id: &str) -> FinanceCommandResult<String> {
    let connection = validate_context(context, app)?;
    let name: String = connection
        .query_row("SELECT name FROM finance_loans WHERE id=?1 AND deleted_at IS NULL", [id], |row| row.get(0))
        .optional()
        .map_err(storage)?
        .ok_or("El préstamo no existe o ya fue eliminado.")?;
    let timestamp = now();
    let transaction = connection.unchecked_transaction().map_err(storage)?;
    transaction
        .execute(
            "UPDATE finance_transactions SET deleted_at=?1,updated_at=?1
             WHERE id IN (SELECT transaction_id FROM finance_loan_installments WHERE loan_id=?2 AND transaction_id IS NOT NULL)",
            params![timestamp, id],
        )
        .map_err(storage)?;
    transaction.execute("DELETE FROM finance_loan_installments WHERE loan_id=?1", [id]).map_err(storage)?;
    transaction
        .execute("UPDATE finance_loans SET deleted_at=?1,active=0,updated_at=?1 WHERE id=?2", params![timestamp, id])
        .map_err(storage)?;
    transaction.commit().map_err(storage)?;
    drop(connection);
    sync_context(context, app)?;
    Ok(format!("Se eliminó el préstamo «{name}» con sus pagos."))
}

fn ensure_loans_category(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO finance_categories (id,name,kind,parent_id,active,description,created_at,updated_at)
             VALUES (?1,?2,'expense',NULL,1,'Cuotas de préstamos.',?3,?3)
             ON CONFLICT(id) DO UPDATE SET active=1",
            params![LOANS_CATEGORY, LOANS_CATEGORY_NAME, now()],
        )
        .map(drop)
        .map_err(storage)
}

/// Pays an installment (the next unpaid one when `payment.number` is
/// empty) with an expense of «Préstamos» from the chosen account.
pub(crate) fn pay_installment(
    app: &crate::host::AppHandle,
    context: &FinanceContext,
    loan_id: &str,
    payment: &Payment,
) -> FinanceCommandResult<(LoanSummary, FinanceTransaction)> {
    let connection = validate_context(context, app)?;
    let summary = load_loan(&connection, loan_id)?.ok_or("El préstamo no existe.")?;
    let installment = match payment.number {
        Some(number) => summary
            .installments
            .iter()
            .find(|item| item.number == number)
            .ok_or_else(|| FinanceCommandError::invalid("Esa cuota no existe en el préstamo."))?,
        None => summary
            .installments
            .iter()
            .find(|item| item.transaction_id.is_none())
            .ok_or_else(|| FinanceCommandError::invalid("El préstamo ya está pagado."))?,
    }
    .clone();
    if installment.transaction_id.is_some() {
        return Err(FinanceCommandError::invalid(format!("La cuota {} ya está pagada.", installment.number)));
    }
    let amount = payment.amount.clone().unwrap_or_else(|| installment.expected_amount.clone());
    if !valid_money(&amount) {
        return Err(FinanceCommandError::invalid("El importe pagado tiene que ser mayor que cero."));
    }
    if !valid_iso_date(&payment.date) {
        return Err(FinanceCommandError::invalid("Elegí la fecha del pago."));
    }
    let account_id = payment
        .account_id
        .clone()
        .filter(|id| !id.is_empty())
        .or_else(|| summary.loan.account_id.clone())
        .ok_or_else(|| FinanceCommandError::invalid("Elegí la cuenta con la que pagaste: el préstamo no tiene una habitual."))?;
    ensure_loans_category(&connection)?;
    drop(connection);
    let expense = FinanceTransaction {
        id: uuid::Uuid::new_v4().to_string(),
        transaction_type: "expense".into(),
        amount: format_cents(parse_cents(&amount).unwrap_or_default()),
        currency: summary.loan.currency.clone(),
        effective_date: payment.date.clone(),
        account_id,
        destination_account_id: None,
        category_id: Some(LOANS_CATEGORY.into()),
        description: format!("Cuota {}/{} de {}", installment.number, summary.loan.installment_count, summary.loan.name),
        source: String::new(),
        status: "confirmed".into(),
        actor_user_id: None,
        actor_library_user_id: Some(context.actor_library_user_id.clone()),
        source_artifact_id: None,
        service_id: None,
        merchant_id: None,
        operation_fingerprint: None,
        installment_id: None,
        source_reference: Some(format!("loan:{loan_id}:{}", installment.number)),
        raw_source: None,
        created_at: None,
        updated_at: None,
        purchase_date: None,
    };
    let saved = crate::finance::finance_save_transaction(
        app.clone(),
        crate::finance::SaveTransactionPayload { context: context.clone(), transaction: expense },
    )?;
    let connection = validate_context(context, app)?;
    let linked = connection.execute(
        "UPDATE finance_loan_installments SET paid_amount=?1,paid_date=?2,transaction_id=?3,updated_at=?4
         WHERE loan_id=?5 AND number=?6 AND transaction_id IS NULL",
        params![saved.amount, payment.date, saved.id, now(), loan_id, installment.number],
    );
    if !matches!(linked, Ok(1)) {
        // The installment changed meanwhile: the expense must not stay alone.
        let _ = connection.execute("UPDATE finance_transactions SET deleted_at=?1 WHERE id=?2", params![now(), saved.id]);
        return Err("La cuota cambió mientras se pagaba; probá de nuevo.".into());
    }
    let summary = load_loan(&connection, loan_id)?.ok_or("El préstamo no existe.")?;
    drop(connection);
    sync_context(context, app)?;
    Ok((summary, saved))
}

/// Takes back the payment of an installment: its expense is deleted.
pub(crate) fn unpay_installment(app: &crate::host::AppHandle, context: &FinanceContext, loan_id: &str, number: u32) -> FinanceCommandResult<LoanSummary> {
    let connection = validate_context(context, app)?;
    let transaction_id: Option<String> = connection
        .query_row(
            "SELECT transaction_id FROM finance_loan_installments WHERE loan_id=?1 AND number=?2",
            params![loan_id, number],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?
        .ok_or("Esa cuota no existe en el préstamo.")?;
    let Some(transaction_id) = transaction_id else {
        return Err(FinanceCommandError::invalid(format!("La cuota {number} no está pagada.")));
    };
    let timestamp = now();
    let transaction = connection.unchecked_transaction().map_err(storage)?;
    transaction
        .execute(
            "UPDATE finance_loan_installments SET paid_amount=NULL,paid_date=NULL,transaction_id=NULL,updated_at=?1 WHERE loan_id=?2 AND number=?3",
            params![timestamp, loan_id, number],
        )
        .map_err(storage)?;
    transaction
        .execute("UPDATE finance_transactions SET deleted_at=?1,updated_at=?1 WHERE id=?2", params![timestamp, transaction_id])
        .map_err(storage)?;
    transaction.commit().map_err(storage)?;
    let summary = load_loan(&connection, loan_id)?.ok_or("El préstamo no existe.")?;
    drop(connection);
    sync_context(context, app)?;
    Ok(summary)
}

fn loan_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Loan> {
    Ok(Loan {
        id: row.get(0)?,
        name: row.get(1)?,
        lender: row.get(2)?,
        currency: row.get(3)?,
        principal: row.get(4)?,
        installment_count: row.get(5)?,
        installment_amount: row.get(6)?,
        first_due_date: row.get(7)?,
        account_id: row.get(8)?,
        notes: row.get(9)?,
        active: row.get::<_, i64>(10)? != 0,
    })
}

const LOAN_COLUMNS: &str = "id,name,lender,currency,principal,installment_count,installment_amount,first_due_date,account_id,notes,active";

fn installments_of(connection: &Connection, loan_id: &str) -> Result<Vec<LoanInstallment>, String> {
    let mut statement = connection
        .prepare(
            "SELECT number,due_date,expected_amount,paid_amount,paid_date,transaction_id
             FROM finance_loan_installments WHERE loan_id=?1 ORDER BY number",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([loan_id], |row| {
            Ok(LoanInstallment {
                number: row.get(0)?,
                due_date: row.get(1)?,
                expected_amount: row.get(2)?,
                paid_amount: row.get(3)?,
                paid_date: row.get(4)?,
                transaction_id: row.get(5)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn summarize(loan: Loan, installments: Vec<LoanInstallment>) -> LoanSummary {
    let cents = |value: &str| parse_cents(value).unwrap_or_default();
    let paid = installments.iter().filter(|item| item.transaction_id.is_some());
    let paid_count = u32::try_from(paid.clone().count()).unwrap_or(u32::MAX);
    let paid_total = paid.map(|item| cents(item.paid_amount.as_deref().unwrap_or(&item.expected_amount))).sum();
    let remaining = installments.iter().filter(|item| item.transaction_id.is_none()).map(|item| cents(&item.expected_amount)).sum();
    let next_due = installments.iter().find(|item| item.transaction_id.is_none()).cloned();
    LoanSummary { loan, installments, paid_count, remaining: format_cents(remaining), paid_total: format_cents(paid_total), next_due }
}

pub(crate) fn load_loan(connection: &Connection, id: &str) -> Result<Option<LoanSummary>, String> {
    let loan = connection
        .query_row(&format!("SELECT {LOAN_COLUMNS} FROM finance_loans WHERE id=?1 AND deleted_at IS NULL"), [id], loan_from_row)
        .optional()
        .map_err(storage)?;
    let Some(loan) = loan else { return Ok(None) };
    let installments = installments_of(connection, &loan.id)?;
    Ok(Some(summarize(loan, installments)))
}

/// Every loan not deleted, the active ones first.
pub(crate) fn list_loans(connection: &Connection) -> Result<Vec<LoanSummary>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT {LOAN_COLUMNS} FROM finance_loans WHERE deleted_at IS NULL ORDER BY active DESC, name"))
        .map_err(storage)?;
    let loans = statement.query_map([], loan_from_row).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?;
    loans
        .into_iter()
        .map(|loan| {
            let installments = installments_of(connection, &loan.id)?;
            Ok(summarize(loan, installments))
        })
        .collect()
}

/// One loan as «Préstamos» of the month shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanRow {
    pub id: String,
    pub name: String,
    pub lender: Option<String>,
    pub currency: String,
    /// Installment due this month, if any.
    pub number: Option<u32>,
    pub expected: Option<String>,
    pub paid: Option<String>,
    /// `paid`, `pending`, `overdue` (an earlier one is unpaid) or `none`.
    pub status: &'static str,
    pub paid_count: u32,
    pub installment_count: u32,
    pub remaining: String,
    pub next_due_date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoansOfMonth {
    pub rows: Vec<LoanRow>,
    /// Paid in the month, per currency (`currency`, `amount`).
    pub paid: Vec<(String, String)>,
    pub pending_count: usize,
}

/// The active loans as the month sees them.
pub(crate) fn loans_of_month(connection: &Connection, month: &str) -> Result<LoansOfMonth, String> {
    let loans = list_loans(connection)?;
    let mut rows = Vec::new();
    let mut paid_by_currency = std::collections::BTreeMap::<String, i128>::new();
    for summary in &loans {
        for item in summary.installments.iter().filter(|item| item.paid_date.as_deref().is_some_and(|date| date.starts_with(month))) {
            *paid_by_currency.entry(summary.loan.currency.clone()).or_default() +=
                parse_cents(item.paid_amount.as_deref().unwrap_or(&item.expected_amount)).unwrap_or_default();
        }
        if !summary.loan.active {
            continue;
        }
        let of_month = summary.installments.iter().find(|item| item.due_date.starts_with(month));
        let overdue = summary
            .installments
            .iter()
            .any(|item| item.transaction_id.is_none() && item.due_date.get(..7).is_some_and(|due| due < month));
        let status = match of_month {
            Some(item) if item.transaction_id.is_some() => "paid",
            _ if overdue => "overdue",
            Some(_) => "pending",
            None => "none",
        };
        rows.push(LoanRow {
            id: summary.loan.id.clone(),
            name: summary.loan.name.clone(),
            lender: summary.loan.lender.clone(),
            currency: summary.loan.currency.clone(),
            number: of_month.map(|item| item.number),
            expected: of_month.map(|item| item.expected_amount.clone()),
            paid: of_month.and_then(|item| item.paid_amount.clone()),
            status,
            paid_count: summary.paid_count,
            installment_count: summary.loan.installment_count,
            remaining: summary.remaining.clone(),
            next_due_date: summary.next_due.as_ref().map(|item| item.due_date.clone()),
        });
    }
    let pending_count = rows.iter().filter(|row| matches!(row.status, "pending" | "overdue")).count();
    Ok(LoansOfMonth { rows, paid: paid_by_currency.into_iter().map(|(currency, cents)| (currency, format_cents(cents))).collect(), pending_count })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loan() -> Loan {
        Loan {
            id: "auto".into(),
            name: "Préstamo del auto".into(),
            lender: Some("Banco".into()),
            currency: "ARS".into(),
            principal: Some("1200000".into()),
            installment_count: 12,
            installment_amount: "125000".into(),
            first_due_date: "2026-01-31".into(),
            account_id: None,
            notes: None,
            active: true,
        }
    }

    #[test]
    fn a_loan_names_the_field_it_lacks() {
        assert_eq!(loan_problem(&loan()), None);
        let field = |change: fn(&mut Loan)| {
            let mut candidate = loan();
            change(&mut candidate);
            loan_problem(&candidate).map(|(field, _)| field)
        };
        assert_eq!(field(|loan| loan.name = " ".into()), Some("name"));
        assert_eq!(field(|loan| loan.currency = "EUR".into()), Some("currency"));
        assert_eq!(field(|loan| loan.installment_count = 0), Some("installmentCount"));
        assert_eq!(field(|loan| loan.installment_amount = "0".into()), Some("installmentAmount"));
        assert_eq!(field(|loan| loan.principal = Some("-5".into())), Some("principal"));
        assert_eq!(field(|loan| loan.first_due_date = "31/01/2026".into()), Some("firstDueDate"));
    }

    #[test]
    fn a_summary_counts_what_was_paid_and_what_is_left() {
        let installment = |number: u32, paid: Option<&str>| LoanInstallment {
            number,
            due_date: format!("2026-{number:02}-28"),
            expected_amount: "100.00".into(),
            paid_amount: paid.map(str::to_string),
            paid_date: paid.map(|_| format!("2026-{number:02}-27")),
            transaction_id: paid.map(|_| format!("t{number}")),
        };
        let summary = summarize(loan(), vec![installment(1, Some("110.50")), installment(2, None), installment(3, None)]);
        assert_eq!(summary.paid_count, 1);
        assert_eq!(summary.paid_total, "110.50", "what was really paid, not what was expected");
        assert_eq!(summary.remaining, "200.00");
        assert_eq!(summary.next_due.map(|item| item.number), Some(2));
    }
}
