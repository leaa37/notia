//! Presupuestos: un tope mensual por categoría de gasto y moneda.
//!
//! Lo gastado es lo que suman los gastos del mes en esa categoría (una
//! cuota de tarjeta cuenta el mes en que se paga). Después de cada cambio
//! en Finanzas se revisan los presupuestos del mes actual: al llegar al
//! 80 % y al pasar el 100 % se le avisa una vez al Owner por Telegram, si
//! el bot corre en este equipo.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::finance::{format_cents, money_text, month_range, now, parse_cents, validate_context, FinanceContext};
use crate::finance_model::cents;

/// Share of the budget that brings the first notice, and the second.
const NEAR: i128 = 80;
const OVER: i128 = 100;

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

fn spending(connection: &Connection, month: &str) -> rusqlite::Result<HashMap<(String, String), i128>> {
    let (from, to) = month_range(month).unwrap_or_default();
    let mut statement = connection.prepare(
        "SELECT category_id,currency,amount FROM finance_movements WHERE kind='expense' AND category_id IS NOT NULL AND date BETWEEN ?1 AND ?2",
    )?;
    let mut totals = HashMap::new();
    for row in statement.query_map(params![from, to], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))? {
        let (category, currency, amount) = row?;
        *totals.entry((category, currency)).or_insert(0) += cents(&amount);
    }
    Ok(totals)
}

fn percent(spent: i128, budget: i128) -> f64 {
    if budget <= 0 {
        return 0.0;
    }
    ((spent as f64 * 1000.0 / budget as f64).round()) / 10.0
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

/// Every budget against what was spent in `month`.
pub(crate) fn budgets_of_month(connection: &Connection, month: &str) -> rusqlite::Result<Vec<BudgetRow>> {
    let spent = spending(connection, month)?;
    let mut statement = connection.prepare(
        "SELECT b.id,b.category_id,c.name,b.currency,b.amount FROM finance_budgets b JOIN finance_categories c ON c.id=b.category_id
         ORDER BY c.name COLLATE NOCASE, b.currency",
    )?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, category_id, category_name, currency, amount)| {
            let budget = cents(&amount);
            let used = spent.get(&(category_id.clone(), currency.clone())).copied().unwrap_or(0);
            BudgetRow {
                id,
                category_id,
                category_name,
                currency,
                budget: format_cents(budget),
                spent: format_cents(used),
                remaining: format_cents(budget - used),
                percent: percent(used, budget),
                status: status(used, budget),
            }
        })
        .collect())
}

fn month_name(month: &str) -> String {
    crate::finance::month_label(month).split(' ').next().unwrap_or("este mes").to_string()
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
            .map_err(|error| error.to_string())?;
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
    let rows = budgets_of_month(&connection, &month).map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())?;
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
    fn the_notice_says_how_much_of_what() {
        let near = alert_text(&row("85000", "100000"), NEAR, "2026-11");
        assert_eq!(near, "Ojo con **Supermercado**: llevás $ 85.000 de $ 100.000 (85 %) del presupuesto de noviembre.");
        let over = alert_text(&row("120000", "100000"), OVER, "2026-11");
        assert!(over.starts_with("Te pasaste del presupuesto de **Supermercado** en noviembre"), "{over}");
    }
}
