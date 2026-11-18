//! Presupuestos: un tope mensual por categoría de gasto y moneda.
//!
//! Lo gastado es lo mismo que «En qué se gastó» cuenta para esa categoría en
//! el mes (gastos confirmados o corregidos; lo que está en tarjeta a pagar
//! todavía no). Después de cada cambio en Finanzas se revisan los
//! presupuestos del mes actual: al llegar al 80 % y al pasar el 100 % se le
//! avisa una vez al Owner por Telegram, si el bot corre en este equipo.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::finance::{format_cents, now, parse_cents, sync_context, validate_context, FinanceCommandError, FinanceCommandResult, FinanceContext};

/// Share of the budget that brings the first notice, and the second.
const NEAR: i128 = 80;
const OVER: i128 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub id: String,
    pub category_id: String,
    pub currency: String,
    pub amount: String,
    pub active: bool,
}

/// A budget with its category's name, for lists and forms.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSummary {
    #[serde(flatten)]
    pub budget: Budget,
    pub category_name: String,
}

/// A budget against what its category spent in the month.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetRow {
    pub id: String,
    pub category_id: String,
    pub category_name: String,
    pub currency: String,
    pub budget: String,
    pub spent: String,
    /// Budget minus spent; negative once it went over.
    pub remaining: String,
    pub percent: f64,
    /// `ok`, `near` (80 % or more) or `over` (more than the budget).
    pub status: &'static str,
}

fn storage(error: rusqlite::Error) -> String {
    format!("No se pudieron leer los presupuestos: {error}")
}

/// What is wrong with `budget`, naming the field.
pub(crate) fn budget_problem(budget: &Budget) -> Option<(&'static str, String)> {
    if budget.id.trim().is_empty() {
        return Some(("id", "Falta el identificador del presupuesto.".into()));
    }
    if budget.category_id.trim().is_empty() {
        return Some(("categoryId", "Elegí la categoría.".into()));
    }
    if !matches!(budget.currency.as_str(), "ARS" | "USD") {
        return Some(("currency", "Elegí pesos o dólares.".into()));
    }
    if !parse_cents(&budget.amount).is_ok_and(|cents| cents > 0) {
        return Some(("amount", "Escribí el tope del mes, mayor que cero.".into()));
    }
    None
}

pub(crate) fn save_budget(app: &crate::host::AppHandle, context: &FinanceContext, budget: &Budget) -> FinanceCommandResult<BudgetSummary> {
    if let Some((_, message)) = budget_problem(budget) {
        return Err(FinanceCommandError::invalid(message));
    }
    let connection = validate_context(context, app)?;
    let category: Option<(String, String)> = connection
        .query_row(
            "SELECT name,kind FROM finance_categories WHERE id=?1 AND active=1",
            [&budget.category_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let category_name = match category {
        None => return Err(FinanceCommandError::invalid("La categoría no existe o está inactiva.")),
        Some((_, kind)) if kind != "expense" => return Err(FinanceCommandError::invalid("Un presupuesto es para una categoría de gastos.")),
        Some((name, _)) => name,
    };
    let taken: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM finance_budgets WHERE category_id=?1 AND currency=?2 AND id<>?3)",
            params![budget.category_id, budget.currency, budget.id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if taken {
        return Err(FinanceCommandError::invalid(format!("«{category_name}» ya tiene un presupuesto en {}: editá ese.", budget.currency)));
    }
    connection
        .execute(
            "INSERT INTO finance_budgets (id,category_id,currency,amount,active,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)
             ON CONFLICT(id) DO UPDATE SET category_id=excluded.category_id,currency=excluded.currency,amount=excluded.amount,
               active=excluded.active,updated_at=excluded.updated_at",
            params![
                budget.id,
                budget.category_id,
                budget.currency,
                format_cents(parse_cents(&budget.amount).unwrap_or_default()),
                budget.active,
                now()
            ],
        )
        .map_err(storage)?;
    drop(connection);
    sync_context(context, app)?;
    Ok(BudgetSummary { budget: budget.clone(), category_name })
}

pub(crate) fn delete_budget(app: &crate::host::AppHandle, context: &FinanceContext, id: &str) -> FinanceCommandResult<String> {
    let connection = validate_context(context, app)?;
    let name: String = connection
        .query_row(
            "SELECT c.name FROM finance_budgets b JOIN finance_categories c ON c.id=b.category_id WHERE b.id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?
        .ok_or("El presupuesto no existe.")?;
    connection.execute("DELETE FROM finance_budget_alerts WHERE budget_id=?1", [id]).map_err(storage)?;
    connection.execute("DELETE FROM finance_budgets WHERE id=?1", [id]).map_err(storage)?;
    drop(connection);
    sync_context(context, app)?;
    Ok(format!("Se eliminó el presupuesto de «{name}»."))
}

pub(crate) fn list_budgets(connection: &Connection) -> Result<Vec<BudgetSummary>, String> {
    let mut statement = connection
        .prepare(
            "SELECT b.id,b.category_id,b.currency,b.amount,b.active,c.name FROM finance_budgets b
             JOIN finance_categories c ON c.id=b.category_id ORDER BY b.active DESC, c.name, b.currency",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([], |row| {
            Ok(BudgetSummary {
                budget: Budget {
                    id: row.get(0)?,
                    category_id: row.get(1)?,
                    currency: row.get(2)?,
                    amount: row.get(3)?,
                    active: row.get::<_, i64>(4)? != 0,
                },
                category_name: row.get(5)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

/// What each expense category spent in `month`, per currency, in cents.
fn spending(connection: &Connection, month: &str) -> Result<HashMap<(String, String), i128>, String> {
    let mut statement = connection
        .prepare(
            "SELECT category_id,currency,amount FROM finance_transactions
             WHERE deleted_at IS NULL AND transaction_type='expense' AND status IN ('confirmed','corrected')
               AND category_id IS NOT NULL AND substr(effective_date,1,7)=?1",
        )
        .map_err(storage)?;
    let mut spent = HashMap::new();
    for row in statement
        .query_map([month], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(storage)?
    {
        let (category, currency, amount) = row.map_err(storage)?;
        *spent.entry((category, currency)).or_default() += parse_cents(&amount).unwrap_or_default();
    }
    Ok(spent)
}

/// Percent of `budget` that `spent` is, rounded to one decimal.
fn percent(spent: i128, budget: i128) -> f64 {
    if budget <= 0 {
        return 0.0;
    }
    ((spent as f64 / budget as f64) * 1000.0).round() / 10.0
}

fn status(spent: i128, budget: i128) -> &'static str {
    if spent > budget {
        "over"
    } else if spent * 100 >= budget * NEAR {
        "near"
    } else {
        "ok"
    }
}

/// The active budgets against `month`, the closest to their limit first.
pub(crate) fn budgets_of_month(connection: &Connection, month: &str) -> Result<Vec<BudgetRow>, String> {
    let spent = spending(connection, month)?;
    let mut rows = list_budgets(connection)?
        .into_iter()
        .filter(|summary| summary.budget.active)
        .map(|summary| {
            let budget = parse_cents(&summary.budget.amount).unwrap_or_default();
            let used = spent.get(&(summary.budget.category_id.clone(), summary.budget.currency.clone())).copied().unwrap_or(0);
            BudgetRow {
                id: summary.budget.id,
                category_id: summary.budget.category_id,
                category_name: summary.category_name,
                currency: summary.budget.currency,
                budget: format_cents(budget),
                spent: format_cents(used),
                remaining: format_cents(budget - used),
                percent: percent(used, budget),
                status: status(used, budget),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| right.percent.total_cmp(&left.percent).then_with(|| left.category_name.cmp(&right.category_name)));
    Ok(rows)
}

/// An amount as people read it: «$ 85.000» or «US$ 1.200,50».
pub(crate) fn money_text(cents: i128, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.abs();
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    let fraction = cents % 100;
    let symbol = if currency == "USD" { "US$" } else { "$" };
    if fraction == 0 {
        format!("{sign}{symbol} {grouped}")
    } else {
        format!("{sign}{symbol} {grouped},{fraction:02}")
    }
}

const MONTHS: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];

fn month_name(month: &str) -> &'static str {
    month.get(5..7).and_then(|value| value.parse::<usize>().ok()).and_then(|value| MONTHS.get(value.wrapping_sub(1))).copied().unwrap_or("este mes")
}

/// The notice of a budget that reached `level` (80 or 100) in `month`.
pub(crate) fn alert_text(row: &BudgetRow, level: i128, month: &str) -> String {
    let spent = money_text(parse_cents(&row.spent).unwrap_or_default(), &row.currency);
    let budget = money_text(parse_cents(&row.budget).unwrap_or_default(), &row.currency);
    let percent = format!("{:.0}", row.percent);
    if level >= OVER {
        format!(
            "Te pasaste del presupuesto de **{}** en {}: llevás {spent} de {budget} ({percent} %).",
            row.category_name,
            month_name(month)
        )
    } else {
        format!(
            "Ojo con **{}**: llevás {spent} de {budget} ({percent} %) del presupuesto de {}.",
            row.category_name,
            month_name(month)
        )
    }
}

/// The level each row reached and has not been told yet, for `month`.
fn pending_alerts(connection: &Connection, rows: &[BudgetRow], month: &str) -> Result<Vec<(BudgetRow, i128)>, String> {
    let mut pending = Vec::new();
    for row in rows {
        let level = match row.status {
            "over" => OVER,
            "near" => NEAR,
            _ => continue,
        };
        let told: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM finance_budget_alerts WHERE budget_id=?1 AND month=?2 AND level>=?3)",
                params![row.id, month, level as i64],
                |result| result.get(0),
            )
            .map_err(storage)?;
        if !told {
            pending.push((row.clone(), level));
        }
    }
    Ok(pending)
}

/// After a change in Finanzas: tells the Owner by Telegram about the
/// budgets of this month that just reached 80 % or went over. A notice is
/// kept as sent only when it reached Telegram. Runs on its own thread, so
/// the change never waits for Telegram.
pub(crate) fn check_alerts_later(app: &crate::host::AppHandle, context: &FinanceContext) {
    let (app, context) = (app.clone(), context.clone());
    let _ = std::thread::Builder::new().name("notia-finance-budgets".into()).spawn(move || {
        if let Err(error) = check_alerts(&app, &context) {
            log::error!("[notia:finance] no se pudieron revisar los presupuestos: {error}");
        }
    });
}

fn check_alerts(app: &crate::host::AppHandle, context: &FinanceContext) -> Result<(), String> {
    let month = chrono::Local::now().format("%Y-%m").to_string();
    let connection = validate_context(context, app)?;
    let rows = budgets_of_month(&connection, &month)?;
    let pending = pending_alerts(&connection, &rows, &month)?;
    if pending.is_empty() {
        return Ok(());
    }
    let Some(library) = crate::library_catalog::catalog_libraries(app).into_iter().find(|library| {
        (!context.library_path.is_empty() && library.path == context.library_path)
            || (context.android_directory_uri.is_some() && library.android_tree_uri == context.android_directory_uri)
    }) else {
        return Ok(());
    };
    let database = crate::library_users::LibraryDatabaseContext {
        library_path: context.library_path.clone(),
        android_directory_uri: context.android_directory_uri.clone(),
    };
    let Ok(Some((_, chat_id))) = crate::library_users::owner_telegram_link(app, &database) else {
        return Ok(());
    };
    for (row, level) in pending {
        let text = alert_text(&row, level, &month);
        if crate::telegram_worker::send_task_message(app, &library.id, chat_id, &text, Vec::new()).is_none() {
            // No bot runs here, or Telegram failed: the next change tries again.
            return Ok(());
        }
        connection
            .execute(
                "INSERT INTO finance_budget_alerts (id,budget_id,month,level,sent_at) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(id) DO NOTHING",
                params![format!("{}:{month}:{level}", row.id), row.id, month, level as i64, now()],
            )
            .map_err(storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(spent: &str, budget: &str) -> BudgetRow {
        let (spent_cents, budget_cents) = (parse_cents(spent).unwrap(), parse_cents(budget).unwrap());
        BudgetRow {
            id: "b".into(),
            category_id: "c".into(),
            category_name: "Supermercado".into(),
            currency: "ARS".into(),
            budget: budget.into(),
            spent: spent.into(),
            remaining: format_cents(budget_cents - spent_cents),
            percent: percent(spent_cents, budget_cents),
            status: status(spent_cents, budget_cents),
        }
    }

    #[test]
    fn a_budget_is_near_from_80_percent_and_over_past_its_limit() {
        assert_eq!(row("79999.99", "100000").status, "ok");
        assert_eq!(row("80000", "100000").status, "near");
        assert_eq!(row("100000", "100000").status, "near", "reaching it is not going over");
        assert_eq!(row("100000.01", "100000").status, "over");
        assert_eq!(row("85000", "100000").percent, 85.0);
    }

    #[test]
    fn amounts_read_as_in_argentina() {
        assert_eq!(money_text(8_500_000, "ARS"), "$ 85.000");
        assert_eq!(money_text(120_050, "USD"), "US$ 1.200,50");
        assert_eq!(money_text(-150_000, "ARS"), "-$ 1.500");
        assert_eq!(money_text(99, "ARS"), "$ 0,99");
    }

    #[test]
    fn the_notice_says_how_much_of_what() {
        let near = alert_text(&row("85000", "100000"), NEAR, "2026-11");
        assert_eq!(near, "Ojo con **Supermercado**: llevás $ 85.000 de $ 100.000 (85 %) del presupuesto de noviembre.");
        let over = alert_text(&row("120000", "100000"), OVER, "2026-11");
        assert!(over.starts_with("Te pasaste del presupuesto de **Supermercado** en noviembre"), "{over}");
    }

    #[test]
    fn a_budget_names_the_field_it_lacks() {
        let budget = Budget { id: "b".into(), category_id: "c".into(), currency: "ARS".into(), amount: "100000".into(), active: true };
        assert_eq!(budget_problem(&budget), None);
        assert_eq!(budget_problem(&Budget { amount: "0".into(), ..budget.clone() }).map(|(field, _)| field), Some("amount"));
        assert_eq!(budget_problem(&Budget { currency: "EUR".into(), ..budget.clone() }).map(|(field, _)| field), Some("currency"));
        assert_eq!(budget_problem(&Budget { category_id: String::new(), ..budget }).map(|(field, _)| field), Some("categoryId"));
    }
}
