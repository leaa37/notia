//! Carga manual de Finanzas: los comandos de la web y de la app de Android.
//!
//! - `finance_session`: el contexto del Owner para una biblioteca, para la
//!   app de Android, que no conoce su carpeta en el servidor.
//! - `finance_entry_menu` y `finance_form`: qué se puede cargar y el
//!   formulario de cada cosa (nuevo, o el registro guardado para editarlo).
//! - `finance_apply`: guarda o borra con [`finance_ops::apply`], lo mismo
//!   que usa el asistente. Un `requestId` ya aplicado devuelve lo mismo sin
//!   repetirlo: la cola sin conexión de la app puede reenviar sin duplicar.
//! - `finance_manage`: lo que se configura, para editarlo o borrarlo.
//!
//! Lo que la persona puede corregir vuelve como resultado (`ok: false` con
//! el mensaje de cada campo), no como error del comando.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::finance::{
    current_month, money_text, month_label, now, read, validate_context, FinanceCommandError, FinanceCommandResult, FinanceContext,
};
use crate::finance_forms::{self as forms, EntryForm};
use crate::finance_model::{self as model, cents};
use crate::finance_ops::{self as ops, OpError, Undo, Values};
use crate::host::{AppHandle, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};

const OWNER: &str = "user-owner";
const MAX_REQUEST_ID_CHARS: usize = 120;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPayload {
    pub library_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormPayload {
    pub context: FinanceContext,
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    /// What the screen preselects (the service of a row), or what is typed
    /// when a field with `reload` changed (with `_changed`).
    #[serde(default)]
    pub prefill: Values,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPayload {
    pub context: FinanceContext,
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    /// `save` (the default) or `delete`.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub values: Values,
    /// The client's id of this load: the same id is applied once.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub ok: bool,
    pub message: String,
    /// What to fix in each field, by its key.
    pub errors: BTreeMap<String, String>,
    pub saved_id: Option<String>,
    /// How to take it back, for the «Deshacer» of the notice.
    pub undo: Option<Undo>,
    /// The request had been applied already: nothing was repeated.
    pub duplicate: bool,
}

/// The Owner's context of `library_id` for the Notia Finanzas app.
pub fn finance_session(app: AppHandle, payload: SessionPayload) -> FinanceCommandResult<FinanceContext> {
    let binding = app
        .state::<LibraryBindingRegistry>()
        .lookup(&payload.library_id)
        .map_err(|_| FinanceCommandError::not_found("La biblioteca no está disponible."))?;
    let context = match binding.root {
        Some(LibraryBindingRoot::Desktop { canonical_root }) => FinanceContext {
            library_path: canonical_root.to_string_lossy().to_string(),
            android_directory_uri: None,
            actor_library_user_id: OWNER.into(),
            source: "android".into(),
        },
        Some(LibraryBindingRoot::Android { tree_uri }) => FinanceContext {
            library_path: String::new(),
            android_directory_uri: Some(tree_uri.as_str().to_string()),
            actor_library_user_id: OWNER.into(),
            source: "android".into(),
        },
        None => return Err(FinanceCommandError::not_found("La biblioteca no está disponible.")),
    };
    validate_context(&context, &app)?;
    Ok(context)
}

/// Who loads: the Notia Finanzas app keeps its origin; the screen's forms
/// are a manual load.
fn source(context: &FinanceContext) -> &'static str {
    if context.source == "android" { "android" } else { "manual" }
}

pub fn finance_entry_menu(_app: AppHandle, _context: FinanceContext) -> FinanceCommandResult<Vec<forms::MenuGroup>> {
    Ok(forms::menu())
}

pub fn finance_form(app: AppHandle, payload: FormPayload) -> FinanceCommandResult<EntryForm> {
    read(&app, &payload.context, |connection| {
        let catalog = forms::catalog(connection)?;
        forms::build(connection, &catalog, &payload.kind, payload.id.as_deref(), &payload.prefill)?
            .ok_or_else(|| FinanceCommandError::not_found(if payload.id.is_some() { "Ya no existe: tal vez se borró antes." } else { "Ese formulario no existe." }))
    })
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_REQUEST_ID_CHARS && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn applied_before(connection: &Connection, request_id: &str) -> rusqlite::Result<Option<ApplyResult>> {
    let stored: Option<String> =
        connection.query_row("SELECT result_json FROM finance_applied_requests WHERE request_id=?1", [request_id], |row| row.get(0)).optional()?;
    Ok(stored.and_then(|json| serde_json::from_str::<ApplyResult>(&json).ok()).map(|result| ApplyResult { duplicate: true, ..result }))
}

pub fn finance_apply(app: AppHandle, payload: ApplyPayload) -> FinanceCommandResult<ApplyResult> {
    let request_id = payload.request_id.as_deref().map(str::trim).filter(|id| !id.is_empty()).map(str::to_string);
    if let Some(request_id) = &request_id {
        if !valid_request_id(request_id) {
            return Err(FinanceCommandError::invalid("El identificador del pedido no es válido."));
        }
        if let Some(result) = read(&app, &payload.context, |connection| Ok(applied_before(connection, request_id)?))? {
            return Ok(result);
        }
    }
    let action = payload.action.clone().unwrap_or_else(|| "save".into());
    let id = payload.id.as_deref().map(str::trim).filter(|id| !id.is_empty()).map(str::to_string);
    let mut connection = validate_context(&payload.context, &app)?;
    let transaction = connection.transaction()?;
    let applied = match ops::apply(&transaction, source(&payload.context), &payload.kind, id.as_deref(), &action, &payload.values) {
        Ok(applied) => applied,
        // Dropping the transaction rolls it back: nothing was written, and
        // the same request id can be sent again once fixed.
        Err(OpError::Rejected { message, errors }) => return Ok(ApplyResult { ok: false, message, errors, ..ApplyResult::default() }),
        Err(OpError::Storage(message)) => return Err(FinanceCommandError::storage(message)),
    };
    let result = ApplyResult { ok: true, message: applied.message, saved_id: applied.saved_id, undo: applied.undo, ..ApplyResult::default() };
    if let Some(request_id) = &request_id {
        let json = serde_json::to_string(&result).map_err(|error| FinanceCommandError::storage(error.to_string()))?;
        transaction.execute(
            "INSERT INTO finance_applied_requests (request_id,kind,result_json,applied_at) VALUES (?1,?2,?3,?4) ON CONFLICT(request_id) DO NOTHING",
            params![request_id, payload.kind, json, now()],
        )?;
    }
    transaction.commit()?;
    drop(connection);
    crate::finance::sync_context(&payload.context, &app)?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Administrar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManageRow {
    pub id: String,
    /// The form that edits it.
    pub kind: &'static str,
    pub title: String,
    pub subtitle: String,
    /// Finished or unused: shown quieter.
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManageSection {
    pub key: &'static str,
    pub title: &'static str,
    pub add_kind: &'static str,
    pub add_label: &'static str,
    pub empty: &'static str,
    pub rows: Vec<ManageRow>,
}

fn manage_row(id: &str, kind: &'static str, title: impl Into<String>, subtitle: impl Into<String>, muted: bool) -> ManageRow {
    ManageRow { id: id.into(), kind, title: title.into(), subtitle: subtitle.into(), muted }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub(crate) fn manage_sections(connection: &Connection) -> rusqlite::Result<Vec<ManageSection>> {
    let month = current_month();
    let purchases = model::purchase_states(connection)?;
    let loans = model::loan_states(connection)?;
    let uses = |sql: &str| -> rusqlite::Result<BTreeMap<String, usize>> {
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().map(|(id, count)| (id, count as usize)).collect())
    };
    let category_uses = uses("SELECT category_id,COUNT(*) FROM finance_movements WHERE category_id IS NOT NULL GROUP BY category_id")?;
    let account_uses = uses("SELECT account_id,COUNT(*) FROM finance_movements WHERE account_id IS NOT NULL GROUP BY account_id")?;

    let cards = model::list_cards(connection)?
        .into_iter()
        .map(|card| {
            let open = purchases.iter().filter(|state| state.purchase.card_id == card.id && !state.finished).collect::<Vec<_>>();
            let mut debt = BTreeMap::<String, i128>::new();
            for state in &open {
                *debt.entry(state.purchase.currency.clone()).or_default() += cents(&state.remaining);
            }
            let subtitle = if open.is_empty() {
                "Sin cuotas pendientes".to_string()
            } else {
                format!("{} · debe {}", plural(open.len(), "compra en curso", "compras en curso"), debt.iter().map(|(currency, amount)| money_text(*amount, currency)).collect::<Vec<_>>().join(" + "))
            };
            manage_row(&card.id, "card", card.name, subtitle, false)
        })
        .collect();
    let mut purchase_rows = purchases
        .iter()
        .filter(|state| !state.finished || crate::finance::months_between(&state.last_month(), &month) <= 1)
        .map(|state| {
            let purchase = &state.purchase;
            let subtitle = if state.finished {
                format!("Pagada · {}", money_text(cents(&purchase.total), &purchase.currency))
            } else {
                format!(
                    "{}/{} cuotas · resta {}",
                    state.paid_count,
                    purchase.installment_count,
                    money_text(cents(&state.remaining), &purchase.currency)
                )
            };
            manage_row(&purchase.id, "card-purchase", purchase.name.clone(), subtitle, state.finished)
        })
        .collect::<Vec<_>>();
    purchase_rows.sort_by_key(|row| row.muted);
    let services = model::services_of_month(connection, &month)?
        .into_iter()
        .map(|service| {
            let subtitle = match (&service.paid, &service.currency) {
                (Some(paid), Some(currency)) => format!("Pagado en {}: {}", month_label(&month).split(' ').next().unwrap_or_default(), money_text(cents(&paid.amount), currency)),
                _ => "Sin pagar este mes".to_string(),
            };
            manage_row(&service.service.id, "service", service.service.name.clone(), subtitle, false)
        })
        .collect();
    let loan_rows = loans
        .iter()
        .map(|state| {
            let loan = &state.loan;
            let subtitle = if state.finished {
                format!("Pagado · {}", money_text(cents(&state.total_paid), &loan.currency))
            } else {
                format!("{}/{} cuotas · pagaste {}", state.paid_count, loan.installment_count, money_text(cents(&state.total_paid), &loan.currency))
            };
            manage_row(&loan.id, "loan", loan.name.clone(), subtitle, state.finished)
        })
        .collect();
    let categories = model::list_categories(connection)?;
    let budget_rows = crate::finance_budgets::budgets_of_month(connection, &month)?
        .into_iter()
        .map(|budget| {
            manage_row(
                &budget.id,
                "budget",
                budget.category_name.clone(),
                format!("{} de {} por mes", money_text(cents(&budget.spent), &budget.currency), money_text(cents(&budget.budget), &budget.currency)),
                false,
            )
        })
        .collect();
    let category_rows = |kind: &str| {
        categories
            .iter()
            .filter(|category| category.kind == kind)
            .map(|category| {
                let count = category_uses.get(&category.id).copied().unwrap_or(0);
                manage_row(&category.id, "category", category.name.clone(), if count == 0 { "Sin movimientos".into() } else { plural(count, "movimiento", "movimientos") }, count == 0)
            })
            .collect::<Vec<_>>()
    };
    let balances = model::reserve_balances(connection)?;
    let reserves = model::list_reserves(connection)?
        .into_iter()
        .map(|reserve| {
            let balance = balances.get(&reserve.id).copied().unwrap_or(0);
            let goal = reserve.goal.as_deref().map(|goal| format!(" de {}", money_text(cents(goal), &reserve.currency))).unwrap_or_default();
            manage_row(&reserve.id, "reserve", reserve.name.clone(), format!("{}{goal}", money_text(balance, &reserve.currency)), false)
        })
        .collect();
    let accounts = model::list_accounts(connection)?
        .into_iter()
        .map(|account| {
            let count = account_uses.get(&account.id).copied().unwrap_or(0);
            manage_row(&account.id, "account", account.name, if count == 0 { "Sin movimientos".into() } else { plural(count, "movimiento", "movimientos") }, false)
        })
        .collect();
    let section = |key, title, add_kind, add_label, empty, rows| ManageSection { key, title, add_kind, add_label, empty, rows };
    Ok(vec![
        section("cards", "Tarjetas", "card", "Nueva tarjeta", "Sin tarjetas. Creá una para cargarle compras en cuotas.", cards),
        section("purchases", "Compras con tarjeta", "card-purchase", "Nueva compra", "Sin compras en cuotas.", purchase_rows),
        section("services", "Servicios", "service", "Nuevo servicio", "Sin servicios.", services),
        section("loans", "Préstamos", "loan", "Nuevo préstamo", "Sin préstamos.", loan_rows),
        section("budgets", "Presupuestos", "budget", "Nuevo presupuesto", "Sin presupuestos.", budget_rows),
        section("expense-categories", "Categorías de gastos", "category", "Nueva categoría", "Sin categorías.", category_rows("expense")),
        section("income-categories", "Categorías de ingresos", "category", "Nueva categoría", "Sin categorías.", category_rows("income")),
        section("reserves", "Reservas de ahorro", "reserve", "Nueva reserva", "Sin reservas.", reserves),
        section("accounts", "Cuentas", "account", "Nueva cuenta", "Sin cuentas. Son opcionales: sirven para anotar de dónde sale la plata.", accounts),
    ])
}

pub fn finance_manage(app: AppHandle, context: FinanceContext) -> FinanceCommandResult<Vec<ManageSection>> {
    read(&app, &context, |connection| Ok(manage_sections(connection)?))
}
