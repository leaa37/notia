//! Carga manual de Finanzas: los comandos de la web y de la app de Android.
//!
//! - `finance_session`: el contexto del Owner para una biblioteca, para la
//!   app de Android, que no conoce su carpeta en el servidor.
//! - `finance_entry_menu` y `finance_form`: qué se puede cargar y el
//!   formulario de cada cosa (nuevo, o el registro guardado para editarlo).
//! - `finance_apply`: guarda, borra o hace la acción de un formulario. Usa
//!   los mismos constructores y funciones que las herramientas del
//!   asistente, así un gasto cargado a mano y uno cargado por el chat son
//!   iguales. Un `requestId` ya aplicado devuelve lo mismo sin repetirlo:
//!   la cola sin conexión de la app puede reenviar sin duplicar.
//! - `finance_manage`: cuentas, categorías, servicios, reservas,
//!   préstamos, presupuestos, planes de cuotas y comercios para editarlos.
//! - `finance_resolve_review`: responde un caso de «Para revisar».
//!
//! Los errores de lo cargado vuelven como resultado (`ok: false` con el
//! mensaje por campo), no como error del comando: la pantalla los muestra
//! junto a cada campo.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::finance::{
    format_cents, parse_cents, validate_context, FinanceAccount, FinanceCategory, FinanceCommandError,
    FinanceCommandResult, FinanceContext, FinanceSavingsReserve, FinanceService, FinanceTransaction,
};
use crate::finance_agent_inputs::{self as inputs, AgentInputError, CreateOrExisting};
use crate::finance_budgets::{self as budgets, money_text, Budget};
use crate::finance_forms::{self as forms, Catalog, Editing, EntryForm, Values};
use crate::finance_loans::{self as loans, Loan};
use crate::host::{AppHandle, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};

const OWNER: &str = "user-owner";
const MAX_REQUEST_ID_CHARS: usize = 120;

// ---------------------------------------------------------------------------
// Payloads and results
// ---------------------------------------------------------------------------

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
    /// What the screen preselects (the service of a row, a reserve).
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
    /// `save` (the default), `delete` or the id of one of the form's actions.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub values: Values,
    /// The client's id of this load: the same id is applied once.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPayload {
    pub context: FinanceContext,
    pub review_item_id: String,
    pub option_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub ok: bool,
    pub message: String,
    /// What to fix in each field, by its key.
    pub errors: BTreeMap<String, String>,
    /// The id of what was saved.
    pub saved_id: Option<String>,
    /// The request had been applied already: nothing was repeated.
    pub duplicate: bool,
}

impl ApplyResult {
    fn saved(message: impl Into<String>, id: Option<String>) -> Self {
        ApplyResult { ok: true, message: message.into(), saved_id: id, ..Self::default() }
    }

    fn invalid(message: impl Into<String>, errors: BTreeMap<String, String>) -> Self {
        ApplyResult { ok: false, message: message.into(), errors, ..Self::default() }
    }

    fn field(key: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self::invalid(message.clone(), BTreeMap::from([(key.to_string(), message)]))
    }
}

/// A domain error that the person can fix becomes a result; a storage or
/// permission failure stays an error of the command.
fn outcome(result: FinanceCommandResult<ApplyResult>) -> FinanceCommandResult<ApplyResult> {
    match result {
        Err(error) if matches!(error.code, "validation" | "conflict" | "notFound") => Ok(ApplyResult::invalid(error.message, BTreeMap::new())),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// The Owner's context of `library_id` for the Notia Finanzas app.
pub fn finance_session(app: AppHandle, payload: SessionPayload) -> FinanceCommandResult<FinanceContext> {
    let binding = app
        .state::<LibraryBindingRegistry>()
        .lookup(&payload.library_id)
        .map_err(|_| FinanceCommandError::from("La biblioteca no está disponible."))?;
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
        None => return Err("La biblioteca no está disponible.".into()),
    };
    validate_context(&context, &app)?;
    Ok(context)
}

/// The context of a load: the Notia Finanzas app keeps its origin; the
/// screen's forms are a manual load.
fn manual(context: &FinanceContext) -> FinanceContext {
    let source = if context.source == "android" { "android" } else { "manual" };
    FinanceContext { source: source.into(), ..context.clone() }
}

// ---------------------------------------------------------------------------
// Catalog and stored records
// ---------------------------------------------------------------------------

fn storage(error: rusqlite::Error) -> String {
    format!("No se pudo leer Finanzas: {error}")
}

fn names(connection: &Connection, table: &str) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection.prepare(&format!("SELECT id,name FROM {table} ORDER BY name COLLATE NOCASE")).map_err(storage)?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn load_catalog(app: &AppHandle, context: &FinanceContext) -> FinanceCommandResult<Catalog> {
    let accounts = crate::finance::finance_list_accounts(app.clone(), context.clone())?;
    let categories = crate::finance::finance_list_categories(app.clone(), context.clone())?;
    let services = crate::finance::finance_list_services(app.clone(), context.clone())?;
    let connection = validate_context(context, app)?;
    Ok(Catalog {
        accounts,
        categories,
        services,
        reserves: crate::finance::finance_list_savings_inner(&connection)?,
        loans: loans::list_loans(&connection)?,
        merchants: names(&connection, "finance_merchants")?,
        products: names(&connection, "finance_products")?,
        today: today(),
    })
}

fn object(value: impl Serialize) -> Values {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn not_found(what: &str) -> FinanceCommandError {
    FinanceCommandError { code: "notFound", message: format!("{what} no existe o ya fue eliminado.") }
}

/// The form a stored movement opens with: its type decides it.
fn movement_kind(transaction: &FinanceTransaction) -> &'static str {
    match transaction.transaction_type.as_str() {
        "income" => "income",
        "transfer" => "transfer",
        _ => "expense",
    }
}

/// The stored record `id` of `kind` as the values of its form, with how
/// the form treats it. `kind` may change: a «movement» opens as the form of
/// its type, and a currency exchange as its savings movement.
fn stored(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, kind: &str, id: &str) -> FinanceCommandResult<(String, Values, Editing)> {
    let editing = |owner| Editing { id: Some(id.to_string()), owner };
    let connection = validate_context(context, app)?;
    match kind {
        "movement" | "expense" | "income" | "transfer" => {
            let transaction = crate::finance::finance_get_transaction(
                app.clone(),
                crate::finance::GetFinanceRecordPayload { context: context.clone(), id: id.into() },
            )?;
            if transaction.transaction_type == "exchange" {
                let movement: Option<String> = connection
                    .query_row("SELECT id FROM finance_savings_movements WHERE linked_transaction_id=?1", [id], |row| row.get(0))
                    .optional()
                    .map_err(storage)?;
                let movement = movement.ok_or_else(|| not_found("El cambio de moneda"))?;
                return stored(app, context, catalog, "savings-movement", &movement);
            }
            let owner = crate::finance::transaction_owner(&connection, id)?;
            let mut values = object(&transaction);
            // A card expense waiting for its statement shows the day it was bought.
            if let Some(date) = transaction.purchase_date.as_ref().filter(|_| transaction.status == crate::finance_matching::CARD_UNPAID) {
                values.insert("effectiveDate".into(), json!(date));
            }
            Ok((movement_kind(&transaction).into(), values, editing(owner)))
        }
        "savings-movement" => {
            let values = connection
                .query_row(
                    "SELECT reserve_id,account_id,movement_type,amount,effective_date,description,reason FROM finance_savings_movements WHERE id=?1",
                    [id],
                    |row| {
                        Ok(json!({
                            "reserveId": row.get::<_, String>(0)?,
                            "accountId": row.get::<_, Option<String>>(1)?,
                            "movementType": row.get::<_, String>(2)?,
                            "amount": row.get::<_, String>(3)?,
                            "effectiveDate": row.get::<_, String>(4)?,
                            "description": row.get::<_, String>(5)?,
                            "reason": row.get::<_, Option<String>>(6)?,
                        }))
                    },
                )
                .optional()
                .map_err(storage)?
                .ok_or_else(|| not_found("El movimiento de ahorro"))?;
            Ok((kind.into(), values.as_object().cloned().unwrap_or_default(), editing(None)))
        }
        "salary" => {
            let mut values = connection
                .query_row(
                    "SELECT employer,period,payment_date,account_id,net_amount,gross_amount,deductions_total FROM finance_salary_receipts WHERE id=?1",
                    [id],
                    |row| {
                        Ok(json!({
                            "employer": row.get::<_, String>(0)?,
                            "period": row.get::<_, String>(1)?,
                            "paymentDate": row.get::<_, String>(2)?,
                            "accountId": row.get::<_, String>(3)?,
                            "netAmount": row.get::<_, String>(4)?,
                            "grossAmount": row.get::<_, String>(5)?,
                            "deductionsTotal": row.get::<_, String>(6)?,
                        }))
                    },
                )
                .optional()
                .map_err(storage)?
                .and_then(|value| value.as_object().cloned())
                .ok_or_else(|| not_found("El sueldo"))?;
            let mut statement = connection
                .prepare("SELECT name,concept_type,amount FROM finance_salary_concepts WHERE salary_receipt_id=?1 ORDER BY rowid")
                .map_err(storage)?;
            let concepts = statement
                .query_map([id], |row| Ok(json!({ "name": row.get::<_, String>(0)?, "conceptType": row.get::<_, String>(1)?, "amount": row.get::<_, String>(2)? })))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            values.insert("concepts".into(), Value::Array(concepts));
            Ok((kind.into(), values, editing(None)))
        }
        "purchase" => {
            let mut values = connection
                .query_row(
                    "SELECT m.name,p.observed_at,t.account_id,p.currency,t.category_id,p.discount_amount,p.total_amount
                     FROM finance_purchases p JOIN finance_transactions t ON t.id=p.transaction_id
                     LEFT JOIN finance_merchants m ON m.id=p.merchant_id WHERE p.id=?1",
                    [id],
                    |row| {
                        Ok(json!({
                            "merchantName": row.get::<_, Option<String>>(0)?,
                            "observedAt": row.get::<_, String>(1)?,
                            "accountId": row.get::<_, String>(2)?,
                            "currency": row.get::<_, String>(3)?,
                            "categoryId": row.get::<_, Option<String>>(4)?,
                            "discountAmount": row.get::<_, Option<String>>(5)?,
                            "totalAmount": row.get::<_, String>(6)?,
                        }))
                    },
                )
                .optional()
                .map_err(storage)?
                .and_then(|value| value.as_object().cloned())
                .ok_or_else(|| not_found("El ticket"))?;
            let items = crate::finance_records::purchase_items(app, context, id)?
                .into_iter()
                .map(|item| {
                    json!({
                        "originalDescription": item["description"],
                        "quantity": item["quantity"],
                        "unitPrice": item["unitPrice"],
                        "lineTotal": item["lineTotal"],
                    })
                })
                .collect();
            values.insert("items".into(), Value::Array(items));
            Ok((kind.into(), values, editing(None)))
        }
        "card-statement" => {
            let statements = crate::finance_records::finance_list_credit_card_statements(
                app.clone(),
                crate::finance_records::ListPeriodPayload { context: context.clone(), from: None, to: None, merchant_id: None, product_id: None },
            )?;
            let statement = statements.into_iter().find(|statement| statement.id == id).ok_or_else(|| not_found("El resumen"))?;
            Ok((kind.into(), object(&statement), editing(None)))
        }
        "installment-plan" => {
            let plans = crate::finance_records::finance_list_installment_plans(app.clone(), context.clone())?;
            let plan = plans.into_iter().find(|summary| summary.plan.id == id).ok_or_else(|| not_found("El plan de cuotas"))?;
            Ok((kind.into(), object(&plan.plan), editing(None)))
        }
        "service-invoice" => {
            let invoices = crate::finance::finance_list_service_invoices(app.clone(), context.clone(), None)?;
            let invoice = invoices.into_iter().find(|invoice| invoice.id == id).ok_or_else(|| not_found("La factura"))?;
            Ok((kind.into(), object(&invoice), editing(None)))
        }
        "account" => {
            let account = catalog.accounts.iter().find(|account| account.id == id).ok_or_else(|| not_found("La cuenta"))?;
            Ok((kind.into(), object(account), editing(None)))
        }
        "category" => {
            let category = catalog.categories.iter().find(|category| category.id == id).ok_or_else(|| not_found("La categoría"))?;
            Ok((kind.into(), object(category), editing(None)))
        }
        "service" => {
            let service = catalog.services.iter().find(|service| service.id == id).ok_or_else(|| not_found("El servicio"))?;
            Ok((kind.into(), object(service), editing(None)))
        }
        "reserve" => {
            let reserve = catalog.reserves.iter().find(|reserve| reserve.id == id).ok_or_else(|| not_found("La reserva"))?;
            Ok((kind.into(), object(reserve), editing(None)))
        }
        "loan" => {
            let loan = catalog.loans.iter().find(|summary| summary.loan.id == id).ok_or_else(|| not_found("El préstamo"))?;
            Ok((kind.into(), object(&loan.loan), editing(None)))
        }
        "budget" => {
            let budget = budgets::list_budgets(&connection)?.into_iter().find(|summary| summary.budget.id == id).ok_or_else(|| not_found("El presupuesto"))?;
            Ok((kind.into(), object(&budget.budget), editing(None)))
        }
        "merchant" | "product" => {
            let list = if kind == "merchant" { &catalog.merchants } else { &catalog.products };
            let name = list.iter().find(|(item, _)| item == id).map(|(_, name)| name.clone()).ok_or_else(|| not_found("El registro"))?;
            Ok((kind.into(), Map::from_iter([("name".to_string(), json!(name))]), editing(None)))
        }
        _ => Err("Ese registro no se puede editar desde un formulario.".into()),
    }
}

/// The form of a payload: a new one with what the screen preselected, or
/// the stored record to edit.
fn form_of(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, kind: &str, id: Option<&str>, prefill: &Values) -> FinanceCommandResult<EntryForm> {
    let (kind, mut values, editing) = match id.filter(|id| !id.trim().is_empty()) {
        Some(id) => stored(app, context, catalog, kind, id)?,
        None => (kind.to_string(), Map::new(), Editing::default()),
    };
    for (key, value) in prefill {
        values.insert(key.clone(), value.clone());
    }
    forms::build(&kind, catalog, &editing, &values).ok_or_else(|| FinanceCommandError::from("Ese tipo de carga no existe."))
}

pub fn finance_entry_menu(_app: AppHandle, _context: FinanceContext) -> FinanceCommandResult<Vec<forms::MenuGroup>> {
    Ok(forms::menu())
}

pub fn finance_form(app: AppHandle, payload: FormPayload) -> FinanceCommandResult<EntryForm> {
    let catalog = load_catalog(&app, &payload.context)?;
    form_of(&app, &payload.context, &catalog, &payload.kind, payload.id.as_deref(), &payload.prefill)
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

fn valid_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_REQUEST_ID_CHARS && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn applied_before(app: &AppHandle, context: &FinanceContext, request_id: &str) -> FinanceCommandResult<Option<ApplyResult>> {
    let connection = validate_context(context, app)?;
    let stored: Option<String> = connection
        .query_row("SELECT result_json FROM finance_applied_requests WHERE request_id=?1", [request_id], |row| row.get(0))
        .optional()
        .map_err(storage)?;
    Ok(stored.and_then(|json| serde_json::from_str::<ApplyResult>(&json).ok()).map(|result| ApplyResult { duplicate: true, ..result }))
}

fn remember_applied(app: &AppHandle, context: &FinanceContext, request_id: &str, kind: &str, result: &ApplyResult) -> FinanceCommandResult<()> {
    let connection = validate_context(context, app)?;
    let json = serde_json::to_string(result).map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO finance_applied_requests (request_id,kind,result_json,applied_at) VALUES (?1,?2,?3,?4) ON CONFLICT(request_id) DO NOTHING",
            params![request_id, kind, json, crate::finance::now()],
        )
        .map_err(storage)?;
    crate::finance::sync_context(context, app)?;
    Ok(())
}

pub fn finance_apply(app: AppHandle, payload: ApplyPayload) -> FinanceCommandResult<ApplyResult> {
    let context = manual(&payload.context);
    let request_id = payload.request_id.as_deref().map(str::trim).filter(|id| !id.is_empty());
    if let Some(request_id) = request_id {
        if !valid_request_id(request_id) {
            return Err("El identificador del pedido no es válido.".into());
        }
        if let Some(result) = applied_before(&app, &context, request_id)? {
            return Ok(result);
        }
    }
    let catalog = load_catalog(&app, &context)?;
    let id = payload.id.as_deref().map(str::trim).filter(|id| !id.is_empty());
    let result = outcome(match payload.action.as_deref().unwrap_or("save") {
        "save" => save(&app, &context, &catalog, &payload.kind, id, &payload.values, request_id),
        "delete" => delete(&app, &context, &payload.kind, id),
        action => run_action(&app, &context, &catalog, &payload.kind, id, action),
    })?;
    if let (Some(request_id), true) = (request_id, result.ok) {
        remember_applied(&app, &context, request_id, &payload.kind, &result)?;
    }
    Ok(result)
}

/// Why a builder rejected the values, for the person (the builders speak
/// to the assistant).
fn input_error(form: &EntryForm, error: &AgentInputError) -> ApplyResult {
    let specific = match error.error {
        "finance-account-currency-mismatch" => Some("La moneda tiene que ser la de la cuenta."),
        "invalid-finance-transfer-account" => Some("Elegí otra cuenta, de la misma moneda."),
        "finance-service-expense-currency-mismatch" => Some("El servicio es de otra moneda."),
        "finance-purchase-account-invalid" | "finance-salary-account-invalid" => Some("La cuenta tiene que ser de la misma moneda."),
        "finance-category-not-found" => Some("Elegí una categoría activa del mismo tipo."),
        "finance-account-not-found" => Some("Elegí una cuenta activa."),
        "finance-service-not-found" => Some("Elegí un servicio activo."),
        _ => None,
    };
    let errors = error
        .invalid_fields
        .iter()
        .map(|key| {
            let key = match key.as_str() {
                "itemAmounts" | "subtotalAmount" => "items",
                other => other,
            };
            let message = specific.map(str::to_string).unwrap_or_else(|| match forms::field_label(form, key) {
                Some(label) => format!("Revisá «{label}»."),
                None => "Revisá este dato.".to_string(),
            });
            (key.to_string(), message)
        })
        .collect::<BTreeMap<_, _>>();
    let message = if errors.len() == 1 { errors.values().next().cloned().unwrap_or_default() } else { "Revisá los datos marcados.".to_string() };
    ApplyResult::invalid(message, errors)
}

fn text(values: &Values, key: &str) -> Option<String> {
    values.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn amount(values: &Values, key: &str) -> Option<String> {
    values.get(key).and_then(|value| inputs::normalize_decimal(value, 2))
}

fn flag(values: &Values, key: &str, default: bool) -> bool {
    values.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn reference(request_id: Option<&str>) -> String {
    format!("manual:{}", request_id.map(str::to_string).unwrap_or_else(new_id))
}

fn account_of<'a>(catalog: &'a Catalog, id: &str) -> Option<&'a FinanceAccount> {
    catalog.accounts.iter().find(|account| account.id == id)
}

fn save(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    kind: &str,
    id: Option<&str>,
    raw: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let form = form_of(app, context, catalog, kind, id, &Map::new())?;
    // What the request leaves out keeps what the form shows: its default,
    // or the stored value of the record being edited.
    let mut raw = raw.clone();
    for field in &form.fields {
        raw.entry(field.key.clone()).or_insert_with(|| field.value.clone());
    }
    let values = forms::clean_values(&form, &raw);
    let missing = forms::missing_required(&form, &values);
    if !missing.is_empty() {
        let message = if missing.len() == 1 { missing[0].1.clone() } else { "Completá los datos marcados.".to_string() };
        return Ok(ApplyResult::invalid(message, missing.into_iter().collect()));
    }
    // A stored movement saves as the form it opened with.
    match form.kind.as_str() {
        "expense" | "income" | "transfer" => save_movement(app, context, catalog, &form, id, &values, request_id),
        "service-payment" => save_service_payment(app, context, catalog, &form, &values, request_id),
        "loan-payment" => save_loan_payment(app, context, &values),
        "savings-movement" => save_savings_movement(app, context, catalog, id, &values),
        "exchange" => save_exchange(app, context, catalog, &values, request_id),
        "salary" => save_salary(app, context, catalog, &form, id, &values, request_id),
        "purchase" => save_purchase(app, context, catalog, &form, id, &values, request_id),
        "card-statement" => save_statement(app, context, catalog, &form, id, &values, request_id),
        "installment-plan" => save_installment_plan(app, context, catalog, id, &values),
        "service-invoice" => save_invoice(app, context, catalog, id, &values),
        "account" => save_account(app, context, id, &values),
        "category" => save_category(app, context, catalog, &form, id, &values),
        "service" => save_service(app, context, catalog, &form, id, &values),
        "reserve" => save_reserve(app, context, catalog, id, &values),
        "loan" => save_loan(app, context, id, &values),
        "budget" => save_budget(app, context, id, &values),
        "merchant" | "product" => save_catalog_entry(app, context, &form.kind, id, &values),
        _ => Err("Ese tipo de carga no existe.".into()),
    }
}

fn save_movement(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    form: &EntryForm,
    id: Option<&str>,
    values: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let mut args = values.clone();
    args.insert("transactionType".into(), json!(form.kind));
    // A card takes pesos and dollars: the builder checks the account's own
    // currency, so another one is set after it.
    let chosen_currency = args.remove("currency").and_then(|value| value.as_str().map(str::to_string));
    let stored = match id {
        Some(id) => Some(crate::finance::finance_get_transaction(
            app.clone(),
            crate::finance::GetFinanceRecordPayload { context: context.clone(), id: id.into() },
        )?),
        None => None,
    };
    if let Some(stored) = stored.as_ref() {
        // What the form shows read-only comes from the stored movement.
        for field in form.fields.iter().filter(|field| field.read_only) {
            if let Some(value) = object(stored).get(&field.key) {
                args.insert(field.key.clone(), value.clone());
            }
        }
    }
    let draft = match inputs::build_transaction(&Value::Object(args), &catalog.accounts, &catalog.categories, &catalog.services, &catalog.today, false) {
        Ok(draft) => draft,
        Err(error) => return Ok(input_error(form, &error)),
    };
    let mut transaction = draft.transaction;
    if let Some(currency) = chosen_currency.filter(|currency| *currency != transaction.currency) {
        let card = account_of(catalog, &transaction.account_id).is_some_and(|account| account.account_type == "credit_card");
        if !card || form.kind == "transfer" {
            return Ok(ApplyResult::field("currency", "Esa cuenta es de otra moneda: solo una tarjeta acepta pesos y dólares."));
        }
        transaction.currency = currency;
    }
    let new = stored.is_none();
    let transaction = match stored {
        Some(stored) => FinanceTransaction {
            amount: transaction.amount,
            currency: transaction.currency,
            effective_date: transaction.effective_date,
            account_id: transaction.account_id,
            destination_account_id: transaction.destination_account_id,
            category_id: transaction.category_id,
            description: transaction.description,
            service_id: transaction.service_id,
            ..stored
        },
        None => FinanceTransaction {
            source_reference: Some(reference(request_id)),
            actor_library_user_id: Some(context.actor_library_user_id.clone()),
            ..transaction
        },
    };
    let (saved, link) = crate::finance::finance_save_transaction_linked(
        app.clone(),
        crate::finance::SaveTransactionPayload { context: context.clone(), transaction },
    )?;
    let mut message = format!(
        "{} guardado: {}.",
        forms::kind_label(&form.kind),
        money_text(parse_cents(&saved.amount).unwrap_or_default(), &saved.currency)
    );
    if saved.status == crate::finance_matching::CARD_UNPAID {
        message.push_str(" Queda en tarjeta, a pagar, hasta que cargues su resumen.");
    }
    if !link.links.is_empty() {
        message.push_str(" Se unió a su línea del resumen.");
    }
    if !link.review_items.is_empty() {
        message.push_str(" Hay algo nuevo en «Para revisar».");
    }
    if let (true, Some(service)) = (new, draft.service) {
        if let Err(error) = crate::finance::record_service_payment(app.clone(), context.clone(), &saved, &service) {
            message.push_str(&format!(" No se pudo marcar el mes del servicio: {}", error.message));
        }
    }
    Ok(ApplyResult::saved(message, Some(saved.id)))
}

fn save_service_payment(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    form: &EntryForm,
    values: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let service_id = text(values, "serviceId").unwrap_or_default();
    let Some(service) = catalog.services.iter().find(|service| service.id == service_id && service.active) else {
        return Ok(ApplyResult::field("serviceId", "Elegí un servicio activo."));
    };
    let Some(account_id) = text(values, "accountId").or_else(|| service.default_account_id.clone()) else {
        return Ok(ApplyResult::field("accountId", "Elegí la cuenta: el servicio no tiene una habitual."));
    };
    let mut args = values.clone();
    args.insert("accountId".into(), json!(account_id));
    args.entry("amount").or_insert_with(|| json!(service.expected_amount));
    args.entry("description").or_insert_with(|| json!(format!("Pago de {}", service.name)));
    args.insert("categoryId".into(), json!(service.category_id));
    let expense_form = EntryForm { kind: "expense".into(), ..form.clone() };
    let mut result = save_movement(app, context, catalog, &expense_form, None, &args, request_id)?;
    if result.ok {
        result.message = result.message.replacen("Gasto guardado", &format!("Pago de {} guardado", service.name), 1);
    }
    Ok(result)
}

fn save_loan_payment(app: &AppHandle, context: &FinanceContext, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let loan_id = text(values, "loanId").unwrap_or_default();
    let payment = loans::Payment {
        number: None,
        amount: match values.get("amount") {
            None => None,
            Some(_) => match amount(values, "amount") {
                Some(value) => Some(value),
                None => return Ok(ApplyResult::field("amount", "Escribí un importe mayor que cero, por ejemplo 125000 o 125.000,50.")),
            },
        },
        date: text(values, "effectiveDate").unwrap_or_else(today),
        account_id: text(values, "accountId"),
    };
    let (summary, saved) = loans::pay_installment(app, context, &loan_id, &payment)?;
    Ok(ApplyResult::saved(
        format!(
            "Cuota pagada: {} ({} de {} pagadas).",
            money_text(parse_cents(&saved.amount).unwrap_or_default(), &saved.currency),
            summary.paid_count,
            summary.loan.installment_count
        ),
        Some(saved.id),
    ))
}

fn reserve_of<'a>(catalog: &'a Catalog, id: &str) -> Option<&'a FinanceSavingsReserve> {
    catalog.reserves.iter().find(|reserve| reserve.id == id)
}

fn save_savings_movement(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let reserve_id = text(values, "reserveId").unwrap_or_default();
    let Some(reserve) = reserve_of(catalog, &reserve_id) else {
        return Ok(ApplyResult::field("reserveId", "Elegí una reserva."));
    };
    let Some(value) = amount(values, "amount") else {
        return Ok(ApplyResult::field("amount", "Escribí un importe mayor que cero."));
    };
    let movement_type = text(values, "movementType").unwrap_or_else(|| "contribution".into());
    if movement_type == "withdrawal" && text(values, "reason").is_none() {
        return Ok(ApplyResult::field("reason", "Un retiro necesita su motivo."));
    }
    let payload = json!({
        "context": context,
        "movement": {
            "id": id.map(str::to_string).unwrap_or_else(new_id),
            "reserveId": reserve.id,
            "accountId": text(values, "accountId"),
            "movementType": movement_type,
            "amount": value,
            "currency": reserve.currency,
            "effectiveDate": text(values, "effectiveDate").unwrap_or_else(today),
            "description": text(values, "description").unwrap_or_default(),
            "reason": text(values, "reason"),
            "source": context.source,
            "status": "confirmed",
            "actorUserId": Value::Null,
            "actorLibraryUserId": context.actor_library_user_id,
            "linkedTransactionId": Value::Null,
        },
    });
    let payload = serde_json::from_value::<crate::finance::SaveSavingsMovementPayload>(payload).map_err(|error| error.to_string())?;
    let saved = crate::finance::finance_save_savings_movement(app.clone(), payload)?;
    Ok(ApplyResult::saved(
        format!("Movimiento guardado en «{}»: {}.", reserve.name, money_text(parse_cents(&saved.amount).unwrap_or_default(), &saved.currency)),
        Some(saved.id),
    ))
}

fn save_exchange(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, values: &Values, request_id: Option<&str>) -> FinanceCommandResult<ApplyResult> {
    let Some(reserve) = reserve_of(catalog, &text(values, "reserveId").unwrap_or_default()) else {
        return Ok(ApplyResult::field("reserveId", "Elegí una reserva."));
    };
    let Some(account) = account_of(catalog, &text(values, "sourceAccountId").unwrap_or_default()) else {
        return Ok(ApplyResult::field("sourceAccountId", "Elegí la cuenta."));
    };
    let (Some(source_amount), Some(savings_amount)) = (amount(values, "sourceAmount"), amount(values, "savingsAmount")) else {
        let key = if amount(values, "sourceAmount").is_none() { "sourceAmount" } else { "savingsAmount" };
        return Ok(ApplyResult::field(key, "Escribí un importe mayor que cero."));
    };
    if account.currency == reserve.currency {
        return Ok(ApplyResult::field("sourceAccountId", "La cuenta y la reserva son de la misma moneda: eso es un movimiento de ahorro."));
    }
    let direction = text(values, "direction").unwrap_or_else(|| "buy".into());
    let payload = json!({
        "context": context,
        "exchange": {
            "id": new_id(),
            "reserveId": reserve.id,
            "sourceAccountId": account.id,
            "sourceAmount": source_amount,
            "sourceCurrency": account.currency,
            "savingsAmount": savings_amount,
            "savingsCurrency": reserve.currency,
            "effectiveDate": text(values, "effectiveDate").unwrap_or_else(today),
            "description": text(values, "description").unwrap_or_else(|| if direction == "sell" { "Venta de dólares".into() } else { "Compra de dólares".into() }),
            "actorUserId": Value::Null,
            "sourceReference": reference(request_id),
            "rawSource": Value::Null,
            "direction": direction,
        },
    });
    let payload = serde_json::from_value::<crate::finance::SaveSavingsExchangePayload>(payload).map_err(|error| error.to_string())?;
    let saved = crate::finance::finance_save_savings_exchange(app.clone(), payload)?;
    Ok(ApplyResult::saved(
        format!(
            "Cambio guardado: {} por {}.",
            money_text(parse_cents(&saved.movement.amount).unwrap_or_default(), &saved.movement.currency),
            money_text(parse_cents(&saved.transaction.amount).unwrap_or_default(), &saved.transaction.currency)
        ),
        Some(saved.movement.id),
    ))
}

fn save_salary(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    form: &EntryForm,
    id: Option<&str>,
    values: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let mut args = values.clone();
    let Some(net) = amount(values, "netAmount") else {
        return Ok(ApplyResult::field("netAmount", "Escribí el neto, mayor que cero."));
    };
    let gross = amount(values, "grossAmount").unwrap_or_else(|| net.clone());
    let deductions = amount(values, "deductionsTotal").unwrap_or_else(|| {
        format_cents((parse_cents(&gross).unwrap_or_default() - parse_cents(&net).unwrap_or_default()).max(0))
    });
    args.insert("grossAmount".into(), json!(gross));
    args.insert("deductionsTotal".into(), json!(deductions));
    let account = account_of(catalog, &text(values, "accountId").unwrap_or_default());
    args.insert("currency".into(), json!(account.map(|account| account.currency.clone()).unwrap_or_default()));
    let mut salary = match inputs::build_salary(&Value::Object(args), &catalog.accounts, &reference(request_id)) {
        Ok(salary) => salary,
        Err(error) => return Ok(input_error(form, &error)),
    };
    if let Some(id) = id {
        salary.id = id.to_string();
    }
    let payload = crate::finance_records::SaveSalaryPayload { context: context.clone(), salary };
    let saved = crate::finance_records::finance_save_salary(app.clone(), payload)?;
    Ok(ApplyResult::saved(
        format!("Sueldo de {} guardado: {} neto.", saved.period, money_text(parse_cents(&saved.net_amount).unwrap_or_default(), &saved.currency)),
        Some(saved.id),
    ))
}

/// `quantity × unit price` rounded to the cent.
fn line_total(quantity: &str, unit_price: &str) -> Option<String> {
    let quantity = quantity.parse::<f64>().ok()?;
    let unit = parse_cents(unit_price).ok()?;
    Some(format_cents((quantity * unit as f64).round() as i128))
}

fn save_purchase(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    form: &EntryForm,
    id: Option<&str>,
    values: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let mut args = values.clone();
    let mut sum = 0_i128;
    let mut items = Vec::new();
    for (index, item) in values.get("items").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().enumerate() {
        let mut item = item.as_object().cloned().unwrap_or_default();
        let quantity = item.get("quantity").and_then(|value| inputs::normalize_decimal(value, 6)).unwrap_or_else(|| "1".into());
        let Some(unit_price) = item.get("unitPrice").and_then(|value| inputs::normalize_decimal(value, 2)) else {
            return Ok(ApplyResult::field("items", format!("Revisá el precio unitario de la fila {}.", index + 1)));
        };
        let total = match item.get("lineTotal") {
            Some(value) => match inputs::normalize_decimal(value, 2) {
                Some(total) => total,
                None => return Ok(ApplyResult::field("items", format!("Revisá el total de la fila {}.", index + 1))),
            },
            None => line_total(&quantity, &unit_price).unwrap_or_default(),
        };
        sum += parse_cents(&total).unwrap_or_default();
        item.insert("quantity".into(), json!(quantity));
        item.insert("unitPrice".into(), json!(unit_price));
        item.insert("lineTotal".into(), json!(total));
        items.push(Value::Object(item));
    }
    args.insert("items".into(), Value::Array(items));
    let discount = amount(values, "discountAmount").map(|value| parse_cents(&value).unwrap_or_default()).unwrap_or(0);
    args.entry("totalAmount").or_insert_with(|| json!(format_cents((sum - discount).max(0))));
    let account = account_of(catalog, &text(values, "accountId").unwrap_or_default());
    if text(values, "currency").is_none() {
        args.insert("currency".into(), json!(account.map(|account| account.currency.clone()).unwrap_or_default()));
    }
    args.insert("observedAt".into(), json!(format!("{}T12:00:00Z", text(values, "observedAt").unwrap_or_else(today))));
    let mut purchase = match inputs::build_purchase(&Value::Object(args), &catalog.accounts, &catalog.categories, &catalog.services, &reference(request_id), &catalog.today) {
        Ok(purchase) => purchase,
        Err(error) => return Ok(input_error(form, &error)),
    };
    if let Some(id) = id {
        purchase.id = id.to_string();
    }
    let saved = crate::finance_records::finance_save_purchase(
        app.clone(),
        crate::finance_records::SavePurchasePayload { context: context.clone(), purchase },
    )?;
    let saved = serde_json::to_value(&saved).unwrap_or_default();
    let id = saved.pointer("/purchase/id").or_else(|| saved.get("id")).and_then(Value::as_str).map(str::to_string);
    Ok(ApplyResult::saved("Ticket guardado con sus productos.", id))
}

fn save_statement(
    app: &AppHandle,
    context: &FinanceContext,
    catalog: &Catalog,
    form: &EntryForm,
    id: Option<&str>,
    values: &Values,
    request_id: Option<&str>,
) -> FinanceCommandResult<ApplyResult> {
    let mut args = values.clone();
    let mut by_type = BTreeMap::<String, i128>::new();
    for item in values.get("items").and_then(Value::as_array).cloned().unwrap_or_default() {
        let kind = item.get("itemType").and_then(Value::as_str).unwrap_or("purchase").to_string();
        let value = item.get("amount").and_then(|value| inputs::normalize_decimal(value, 2)).map(|value| parse_cents(&value).unwrap_or_default()).unwrap_or(0);
        *by_type.entry(kind).or_default() += value;
    }
    let total = |kind: &str| json!(format_cents(by_type.get(kind).copied().unwrap_or(0)));
    for (field, kind) in [("purchasesAmount", "purchase"), ("feesAmount", "fee"), ("interestAmount", "interest"), ("taxesAmount", "tax"), ("creditsAmount", "credit")] {
        args.insert(field.into(), total(kind));
    }
    args.entry("paymentsAmount").or_insert_with(|| total("payment"));
    args.entry("previousBalance").or_insert_with(|| json!("0"));
    if text(values, "issuer").is_none() {
        if let Some(account) = account_of(catalog, &text(values, "accountId").unwrap_or_default()) {
            args.insert("issuer".into(), json!(account.name));
        }
    }
    let mut statement = match inputs::build_credit_card_statement(&Value::Object(args), &catalog.accounts, &reference(request_id)) {
        Ok(statement) => statement,
        Err(error) => return Ok(input_error(form, &error)),
    };
    if let Some(id) = id {
        statement.id = id.to_string();
    }
    let saved = crate::finance_records::finance_save_credit_card_statement(
        app.clone(),
        crate::finance_records::SaveCreditCardStatementPayload { context: context.clone(), statement },
    )?;
    let saved = serde_json::to_value(&saved).unwrap_or_default();
    let id = saved.pointer("/statement/id").and_then(Value::as_str).map(str::to_string);
    let mut message = "Resumen guardado.".to_string();
    if saved.get("reviewItems").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) {
        message.push_str(" Hay líneas en «Para revisar».");
    }
    Ok(ApplyResult::saved(message, id))
}

fn save_installment_plan(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let Some(total) = amount(values, "totalAmount") else {
        return Ok(ApplyResult::field("totalAmount", "Escribí el total, mayor que cero."));
    };
    let count = values.get("installmentCount").and_then(Value::as_u64).and_then(|value| u16::try_from(value).ok()).unwrap_or(0);
    if count == 0 || count > 120 {
        return Ok(ApplyResult::field("installmentCount", "Las cuotas van de 1 a 120."));
    }
    let account_id = text(values, "accountId").unwrap_or_default();
    if account_of(catalog, &account_id).is_none() {
        return Ok(ApplyResult::field("accountId", "Elegí la tarjeta o cuenta."));
    }
    let plan = crate::finance_records::InstallmentPlan {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        account_id,
        merchant_name: text(values, "merchantName").unwrap_or_default(),
        description: text(values, "description").unwrap_or_default(),
        purchase_date: text(values, "purchaseDate").unwrap_or_else(today),
        currency: text(values, "currency").unwrap_or_else(|| "ARS".into()),
        total_amount: total,
        installment_count: count,
    };
    let plan_id = plan.id.clone();
    crate::finance_records::finance_save_installment_plan(app.clone(), crate::finance_records::SaveInstallmentPayload { context: context.clone(), plan })?;
    Ok(ApplyResult::saved(format!("Plan de {count} cuotas guardado."), Some(plan_id)))
}

fn save_invoice(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let Some(service) = catalog.services.iter().find(|service| Some(&service.id) == text(values, "serviceId").as_ref()) else {
        return Ok(ApplyResult::field("serviceId", "Elegí el servicio."));
    };
    let Some(value) = amount(values, "amount") else {
        return Ok(ApplyResult::field("amount", "Escribí el importe, mayor que cero."));
    };
    let payload = json!({
        "context": context,
        "invoice": {
            "id": id.map(str::to_string).unwrap_or_else(new_id),
            "serviceId": service.id,
            "period": text(values, "period").unwrap_or_default(),
            "dueDate": text(values, "dueDate"),
            "provider": text(values, "provider").or_else(|| service.provider.clone()),
            "amount": value,
            "currency": service.currency,
            "transactionId": Value::Null,
            "artifactId": Value::Null,
            "validationStatus": "valid",
            "sourceReference": Value::Null,
            "rawExtraction": Value::Null,
            "createdAt": Value::Null,
            "updatedAt": Value::Null,
        },
    });
    let payload = serde_json::from_value::<crate::finance::SaveFinanceServiceInvoicePayload>(payload).map_err(|error| error.to_string())?;
    let saved = crate::finance::finance_save_service_invoice(app.clone(), payload)?;
    Ok(ApplyResult::saved(format!("Factura de {} guardada.", service.name), Some(saved.id)))
}

fn save_account(app: &AppHandle, context: &FinanceContext, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let account = FinanceAccount {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: text(values, "name").unwrap_or_default(),
        account_type: text(values, "accountType").unwrap_or_else(|| "bank".into()),
        currency: text(values, "currency").unwrap_or_else(|| "ARS".into()),
        active: flag(values, "active", true),
    };
    if !matches!(account.account_type.as_str(), "cash" | "bank" | "credit_card" | "wallet" | "other") {
        return Ok(ApplyResult::field("accountType", "Elegí el tipo de cuenta."));
    }
    let saved = crate::finance::finance_save_account(app.clone(), crate::finance::SaveAccountPayload { context: context.clone(), account })?;
    Ok(ApplyResult::saved(format!("Cuenta «{}» guardada.", saved.name), Some(saved.id)))
}

fn save_category(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, form: &EntryForm, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let category = match id {
        Some(id) => FinanceCategory {
            id: id.to_string(),
            name: text(values, "name").unwrap_or_default(),
            kind: text(values, "kind").unwrap_or_else(|| "expense".into()),
            active: flag(values, "active", true),
            parent_id: catalog.categories.iter().find(|category| category.id == id).and_then(|category| category.parent_id.clone()),
            description: text(values, "description"),
        },
        None => match inputs::build_category(&Value::Object(values.clone()), &catalog.categories) {
            Ok(CreateOrExisting::Create(category)) => category,
            Ok(CreateOrExisting::Existing(category)) => {
                return Ok(ApplyResult::field("name", format!("Ya existe la categoría «{}».", category.name)));
            }
            Err(error) => return Ok(input_error(form, &error)),
        },
    };
    let saved = crate::finance::finance_save_category(app.clone(), crate::finance::SaveCategoryPayload { context: context.clone(), category })?;
    Ok(ApplyResult::saved(format!("Categoría «{}» guardada.", saved.name), Some(saved.id)))
}

fn save_service(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, form: &EntryForm, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let due_day = values.get("dueDay").and_then(Value::as_i64);
    if due_day.is_some_and(|day| !(1..=31).contains(&day)) {
        return Ok(ApplyResult::field("dueDay", "El día va de 1 a 31."));
    }
    let service = match id {
        Some(id) => {
            let stored = catalog.services.iter().find(|service| service.id == id).cloned().ok_or_else(|| not_found("El servicio"))?;
            let Some(expected) = amount(values, "expectedAmount") else {
                return Ok(ApplyResult::field("expectedAmount", "Escribí el importe esperado, mayor que cero."));
            };
            FinanceService {
                name: text(values, "name").unwrap_or_default(),
                category_id: text(values, "categoryId").unwrap_or_default(),
                currency: text(values, "currency").unwrap_or_else(|| "ARS".into()),
                expected_amount: expected,
                due_day,
                default_account_id: text(values, "defaultAccountId"),
                provider: text(values, "provider"),
                modality: text(values, "modality").unwrap_or_else(|| "fixed".into()),
                active: flag(values, "active", true),
                ..stored
            }
        }
        None => match inputs::build_service(&Value::Object(values.clone()), &catalog.categories, &catalog.services) {
            Ok(CreateOrExisting::Create(service)) => service,
            Ok(CreateOrExisting::Existing(service)) => {
                return Ok(ApplyResult::field("name", format!("Ya existe el servicio «{}».", service.name)));
            }
            Err(error) => return Ok(input_error(form, &error)),
        },
    };
    let saved = crate::finance::finance_save_service(app.clone(), crate::finance::SaveFinanceServicePayload { context: context.clone(), service })?;
    if id.is_some() {
        // The active switch has its own rule in the domain.
        let active = flag(values, "active", true);
        if saved.active != active {
            crate::finance::finance_set_service_active(
                app.clone(),
                crate::finance::SetFinanceServiceActivePayload { context: context.clone(), id: saved.id.clone(), active },
            )?;
        }
    }
    Ok(ApplyResult::saved(format!("Servicio «{}» guardado.", saved.name), Some(saved.id)))
}

fn save_reserve(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let opening = match values.get("openingBalance") {
        None => "0".to_string(),
        Some(value) => match inputs::normalize_decimal(value, 2) {
            Some(value) => value,
            None => return Ok(ApplyResult::field("openingBalance", "Escribí el saldo inicial, cero o más.")),
        },
    };
    let balance = id.and_then(|id| reserve_of(catalog, id)).map(|reserve| reserve.balance.clone()).unwrap_or_else(|| opening.clone());
    let reserve = FinanceSavingsReserve {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: text(values, "name").unwrap_or_default(),
        currency: text(values, "currency").unwrap_or_else(|| "USD".into()),
        opening_balance: opening,
        objective: text(values, "objective"),
        active: flag(values, "active", true),
        balance,
    };
    let saved = crate::finance::finance_save_savings_reserve(app.clone(), crate::finance::SaveSavingsReservePayload { context: context.clone(), reserve })?;
    Ok(ApplyResult::saved(format!("Reserva «{}» guardada.", saved.name), Some(saved.id)))
}

fn save_loan(app: &AppHandle, context: &FinanceContext, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let money = |key: &str| amount(values, key);
    let loan = Loan {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: text(values, "name").unwrap_or_default(),
        lender: text(values, "lender"),
        currency: text(values, "currency").unwrap_or_else(|| "ARS".into()),
        // Something typed that is not an amount stays, so the rule names it.
        principal: values.get("principal").map(|_| money("principal").unwrap_or_default()),
        installment_count: values.get("installmentCount").and_then(Value::as_u64).and_then(|value| u32::try_from(value).ok()).unwrap_or(0),
        installment_amount: money("installmentAmount").unwrap_or_default(),
        first_due_date: text(values, "firstDueDate").unwrap_or_default(),
        account_id: text(values, "accountId"),
        notes: text(values, "notes"),
        active: flag(values, "active", true),
    };
    if let Some((field, message)) = loans::loan_problem(&loan) {
        return Ok(ApplyResult::field(field, message));
    }
    let saved = loans::save_loan(app, context, &loan)?;
    Ok(ApplyResult::saved(
        format!("Préstamo «{}» guardado: {} cuotas de {}.", saved.loan.name, saved.loan.installment_count, money_text(parse_cents(&saved.loan.installment_amount).unwrap_or_default(), &saved.loan.currency)),
        Some(saved.loan.id),
    ))
}

fn save_budget(app: &AppHandle, context: &FinanceContext, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let budget = Budget {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        category_id: text(values, "categoryId").unwrap_or_default(),
        currency: text(values, "currency").unwrap_or_else(|| "ARS".into()),
        amount: amount(values, "amount").unwrap_or_default(),
        active: flag(values, "active", true),
    };
    if let Some((field, message)) = budgets::budget_problem(&budget) {
        return Ok(ApplyResult::field(field, message));
    }
    let saved = budgets::save_budget(app, context, &budget)?;
    Ok(ApplyResult::saved(
        format!("Presupuesto de «{}» guardado: {} por mes.", saved.category_name, money_text(parse_cents(&saved.budget.amount).unwrap_or_default(), &saved.budget.currency)),
        Some(saved.budget.id),
    ))
}

fn save_catalog_entry(app: &AppHandle, context: &FinanceContext, kind: &str, id: Option<&str>, values: &Values) -> FinanceCommandResult<ApplyResult> {
    let Some(id) = id else {
        return Err(FinanceCommandError::invalid("Los comercios y productos nacen de los tickets: acá solo se renombran o se unen."));
    };
    let (rename, merge) = if kind == "merchant" { ("rename-merchant", "merge-merchants") } else { ("rename-product", "merge-products") };
    if let Some(into) = text(values, "intoId") {
        crate::finance_edits::finance_edit_catalog(app.clone(), context.clone(), merge.into(), id.into(), into.clone())?;
        return Ok(ApplyResult::saved("Unido: sus compras y precios pasaron al elegido.", Some(into)));
    }
    let name = text(values, "name").unwrap_or_default();
    crate::finance_edits::finance_edit_catalog(app.clone(), context.clone(), rename.into(), id.into(), name.clone())?;
    Ok(ApplyResult::saved(format!("Ahora se llama «{name}»."), Some(id.into())))
}

fn delete(app: &AppHandle, context: &FinanceContext, kind: &str, id: Option<&str>) -> FinanceCommandResult<ApplyResult> {
    let id = id.ok_or_else(|| FinanceCommandError::from("Falta qué eliminar."))?.to_string();
    let entity = |payload_id: String| crate::finance::DeleteFinanceEntityPayload { context: context.clone(), id: payload_id };
    let record = |entity: &str| crate::finance_edits::finance_delete_record(app.clone(), context.clone(), entity.into(), id.clone()).map(|deleted| deleted.summary);
    let message = match kind {
        "movement" | "expense" | "income" | "transfer" | "service-payment" => {
            crate::finance::finance_delete_transaction(app.clone(), entity(id.clone()))?;
            "Movimiento eliminado.".to_string()
        }
        "account" => {
            crate::finance::finance_delete_account(app.clone(), entity(id.clone()))?;
            "Cuenta desactivada.".to_string()
        }
        "category" => {
            crate::finance::finance_delete_category(app.clone(), entity(id.clone()))?;
            "Categoría desactivada.".to_string()
        }
        "savings-movement" | "salary" | "purchase" | "card-statement" | "installment-plan" | "service-invoice" | "service" => record(kind)?,
        "reserve" => record("savings-reserve")?,
        "loan" => loans::delete_loan(app, context, &id)?,
        "budget" => budgets::delete_budget(app, context, &id)?,
        _ => return Err("Eso no se puede eliminar desde acá.".into()),
    };
    Ok(ApplyResult::saved(message, None))
}

fn run_action(app: &AppHandle, context: &FinanceContext, catalog: &Catalog, kind: &str, id: Option<&str>, action: &str) -> FinanceCommandResult<ApplyResult> {
    match (kind, action, id) {
        ("loan", "unpay-last", Some(id)) => {
            let summary = catalog.loans.iter().find(|summary| summary.loan.id == id).ok_or_else(|| not_found("El préstamo"))?;
            let last = summary
                .installments
                .iter()
                .rev()
                .find(|item| item.transaction_id.is_some())
                .ok_or_else(|| FinanceCommandError::invalid("El préstamo no tiene cuotas pagadas."))?;
            let summary = loans::unpay_installment(app, context, id, last.number)?;
            Ok(ApplyResult::saved(format!("Cuota {} desmarcada ({} pagadas).", last.number, summary.paid_count), Some(id.into())))
        }
        _ => Err("Esa acción no existe para este formulario.".into()),
    }
}

// ---------------------------------------------------------------------------
// Review items
// ---------------------------------------------------------------------------

pub fn finance_resolve_review(app: AppHandle, payload: ReviewPayload) -> FinanceCommandResult<ApplyResult> {
    let context = manual(&payload.context);
    outcome(
        crate::finance_edits::finance_resolve_review_item(app, context, payload.review_item_id, payload.option_id)
            .map(|_| ApplyResult::saved("Listo, quedó resuelto.", None)),
    )
}

// ---------------------------------------------------------------------------
// Manage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManageRow {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub subtitle: String,
    pub inactive: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManageSection {
    pub key: &'static str,
    pub title: &'static str,
    /// The form that adds one, when they are added here.
    pub add_kind: Option<&'static str>,
    pub add_label: Option<&'static str>,
    pub empty: &'static str,
    pub rows: Vec<ManageRow>,
}

fn row(id: &str, kind: &'static str, title: impl Into<String>, subtitle: impl Into<String>, inactive: bool) -> ManageRow {
    ManageRow { id: id.into(), kind, title: title.into(), subtitle: subtitle.into(), inactive }
}

fn date_text(date: &str) -> String {
    match (date.get(8..10), date.get(5..7), date.get(..4)) {
        (Some(day), Some(month), Some(year)) => format!("{day}/{month}/{year}"),
        _ => date.into(),
    }
}

/// Id, period, name, amount and currency of a stored document.
type DocumentRow = (String, String, String, String, String);

/// The five text columns of each row of `query`.
fn documents(connection: &Connection, query: &str) -> Result<Vec<DocumentRow>, String> {
    let mut statement = connection.prepare(query).map_err(storage)?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

/// What can be edited, section by section.
pub(crate) fn manage_sections(connection: &Connection, catalog: &Catalog) -> Result<Vec<ManageSection>, String> {
    let money = |value: &str, currency: &str| money_text(parse_cents(value).unwrap_or_default(), currency);
    let account_type = |kind: &str| match kind {
        "credit_card" => "Tarjeta de crédito",
        "cash" => "Efectivo",
        "wallet" => "Billetera virtual",
        "bank" => "Cuenta bancaria",
        _ => "Otra cuenta",
    };
    let category_name = |id: &str| catalog.categories.iter().find(|category| category.id == id).map(|category| category.name.clone()).unwrap_or_default();
    let mut categories = catalog.categories.clone();
    categories.sort_by(|left, right| left.kind.cmp(&right.kind).then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())));
    let plans = {
        let mut statement = connection
            .prepare("SELECT id,description,installment_count,total_amount,currency FROM finance_installment_plans ORDER BY purchase_date DESC")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    Ok(vec![
        ManageSection {
            key: "accounts",
            title: "Cuentas",
            add_kind: Some("account"),
            add_label: Some("Nueva cuenta"),
            empty: "Todavía no hay cuentas.",
            rows: catalog.accounts.iter().map(|account| row(&account.id, "account", &account.name, format!("{} · {}", account_type(&account.account_type), account.currency), !account.active)).collect(),
        },
        ManageSection {
            key: "categories",
            title: "Categorías",
            add_kind: Some("category"),
            add_label: Some("Nueva categoría"),
            empty: "Todavía no hay categorías.",
            rows: categories
                .iter()
                .map(|category| {
                    let kind = if category.kind == "income" { "Ingresos" } else { "Gastos" };
                    let subtitle = match category.description.as_deref().filter(|text| !text.is_empty()) {
                        Some(description) => format!("{kind} · {description}"),
                        None => kind.into(),
                    };
                    row(&category.id, "category", &category.name, subtitle, !category.active)
                })
                .collect(),
        },
        ManageSection {
            key: "services",
            title: "Servicios",
            add_kind: Some("service"),
            add_label: Some("Nuevo servicio"),
            empty: "Todavía no hay servicios.",
            rows: catalog
                .services
                .iter()
                .map(|service| {
                    let due = service.due_day.map(|day| format!(" · vence el {day}")).unwrap_or_default();
                    row(&service.id, "service", &service.name, format!("{}{due} · {}", money(&service.expected_amount, &service.currency), category_name(&service.category_id)), !service.active)
                })
                .collect(),
        },
        ManageSection {
            key: "reserves",
            title: "Reservas de ahorro",
            add_kind: Some("reserve"),
            add_label: Some("Nueva reserva"),
            empty: "Todavía no hay reservas.",
            rows: catalog
                .reserves
                .iter()
                .map(|reserve| {
                    let objective = reserve.objective.as_deref().map(|objective| format!(" · {objective}")).unwrap_or_default();
                    row(&reserve.id, "reserve", &reserve.name, format!("{}{objective}", money(&reserve.balance, &reserve.currency)), !reserve.active)
                })
                .collect(),
        },
        ManageSection {
            key: "loans",
            title: "Préstamos",
            add_kind: Some("loan"),
            add_label: Some("Nuevo préstamo"),
            empty: "No hay préstamos.",
            rows: catalog
                .loans
                .iter()
                .map(|summary| {
                    let next = summary.next_due.as_ref().map(|item| format!(" · próxima {}", date_text(&item.due_date))).unwrap_or_else(|| " · pagado".into());
                    row(
                        &summary.loan.id,
                        "loan",
                        &summary.loan.name,
                        format!("{}/{} pagadas · resta {}{next}", summary.paid_count, summary.loan.installment_count, money(&summary.remaining, &summary.loan.currency)),
                        !summary.loan.active,
                    )
                })
                .collect(),
        },
        ManageSection {
            key: "budgets",
            title: "Presupuestos",
            add_kind: Some("budget"),
            add_label: Some("Nuevo presupuesto"),
            empty: "No hay presupuestos.",
            rows: budgets::list_budgets(connection)?
                .into_iter()
                .map(|summary| row(&summary.budget.id, "budget", &summary.category_name, format!("{} por mes", money(&summary.budget.amount, &summary.budget.currency)), !summary.budget.active))
                .collect(),
        },
        ManageSection {
            key: "installment-plans",
            title: "Planes de cuotas",
            add_kind: Some("installment-plan"),
            add_label: Some("Nuevo plan"),
            empty: "No hay planes de cuotas.",
            rows: plans
                .iter()
                .map(|(id, description, count, total, currency)| row(id, "installment-plan", description, format!("{count} cuotas · {}", money(total, currency)), false))
                .collect(),
        },
        ManageSection {
            key: "salaries",
            title: "Sueldos",
            add_kind: Some("salary"),
            add_label: Some("Nuevo sueldo"),
            empty: "Todavía no hay recibos de sueldo.",
            rows: documents(
                connection,
                "SELECT id,period,employer,net_amount,currency FROM finance_salary_receipts ORDER BY period DESC, payment_date DESC LIMIT 36",
            )?
            .into_iter()
            .map(|(id, period, employer, net, currency)| row(&id, "salary", format!("{period} · {employer}"), format!("{} neto", money(&net, &currency)), false))
            .collect(),
        },
        ManageSection {
            key: "card-statements",
            title: "Resúmenes de tarjeta",
            add_kind: Some("card-statement"),
            add_label: Some("Nuevo resumen"),
            empty: "Todavía no hay resúmenes.",
            rows: documents(
                connection,
                "SELECT s.id,s.period,COALESCE(a.name,s.issuer),s.total_due,s.currency FROM finance_credit_card_statements s
                 LEFT JOIN finance_accounts a ON a.id=s.account_id ORDER BY s.due_date DESC LIMIT 36",
            )?
            .into_iter()
            .map(|(id, period, card, total, currency)| row(&id, "card-statement", format!("{card} · {period}"), format!("{} a pagar", money(&total, &currency)), false))
            .collect(),
        },
        ManageSection {
            key: "merchants",
            title: "Comercios",
            add_kind: None,
            add_label: None,
            empty: "Los comercios aparecen con los tickets.",
            rows: catalog.merchants.iter().map(|(id, name)| row(id, "merchant", name, "Renombrar o unir con otro", false)).collect(),
        },
    ])
}

pub fn finance_manage(app: AppHandle, context: FinanceContext) -> FinanceCommandResult<Vec<ManageSection>> {
    let catalog = load_catalog(&app, &context)?;
    let connection = validate_context(&context, &app)?;
    Ok(manage_sections(&connection, &catalog)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library on disk with its database, and the app around it.
    fn library() -> (AppHandle, FinanceContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("notia-finance-entry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".notia")).expect("library");
        let app = crate::create_app(crate::host::AppPaths::new(Some(root.join("app")), None), crate::host::HostPorts::default());
        let context = FinanceContext { library_path: root.display().to_string(), android_directory_uri: None, actor_library_user_id: OWNER.into(), source: "app".into() };
        (app, context, root)
    }

    fn apply(app: &AppHandle, context: &FinanceContext, kind: &str, id: Option<&str>, values: Value, request_id: Option<&str>) -> ApplyResult {
        finance_apply(
            app.clone(),
            ApplyPayload {
                context: context.clone(),
                kind: kind.into(),
                id: id.map(str::to_string),
                action: None,
                values: values.as_object().cloned().unwrap_or_default(),
                request_id: request_id.map(str::to_string),
            },
        )
        .unwrap_or_else(|error| panic!("{kind}: {}", error.message))
    }

    fn saved(result: ApplyResult) -> String {
        assert!(result.ok, "{result:?}");
        result.saved_id.expect("saved id")
    }

    fn overview(app: &AppHandle, context: &FinanceContext, month: &str) -> Value {
        let payload = serde_json::from_value(json!({ "context": context, "month": month })).expect("payload");
        serde_json::to_value(crate::finance_screen::finance_overview(app.clone(), payload).expect("overview")).expect("json")
    }

    #[test]
    fn everything_the_assistant_loads_can_be_loaded_by_hand() {
        let (app, context, root) = library();
        let month = today()[..7].to_string();
        let bank = saved(apply(&app, &context, "account", None, json!({ "name": "Galicia", "accountType": "bank", "currency": "ARS" }), None));
        let card = saved(apply(&app, &context, "account", None, json!({ "name": "Visa", "accountType": "credit_card", "currency": "ARS" }), None));
        let categories = crate::finance::finance_list_categories(app.clone(), context.clone()).expect("categories");
        let food = categories.iter().find(|category| category.kind == "expense").expect("a default expense category").id.clone();

        // A plain expense, typed as people type amounts.
        let expense = saved(apply(&app, &context, "expense", None, json!({ "amount": "1.500,50", "accountId": bank, "categoryId": food, "description": "Coto" }), Some("pedido-1")));
        let stored = crate::finance::finance_get_transaction(app.clone(), crate::finance::GetFinanceRecordPayload { context: context.clone(), id: expense.clone() }).expect("stored");
        assert_eq!((parse_cents(&stored.amount), stored.source.as_str(), stored.status.as_str()), (Ok(150_050), "manual", "confirmed"));

        // The same request again repeats nothing.
        let again = apply(&app, &context, "expense", None, json!({ "amount": "1.500,50", "accountId": bank, "categoryId": food, "description": "Coto" }), Some("pedido-1"));
        assert!(again.duplicate && again.ok);
        assert_eq!(again.saved_id.as_deref(), Some(expense.as_str()));
        let all = crate::finance::finance_list_all_transactions(app.clone(), context.clone()).expect("all");
        assert_eq!(all.iter().filter(|transaction| transaction.description == "Coto").count(), 1, "the queue cannot duplicate");

        // What is missing is named by field, without failing the command.
        let missing = apply(&app, &context, "expense", None, json!({ "accountId": bank, "description": "Sin importe" }), None);
        assert!(!missing.ok);
        assert!(missing.errors.contains_key("amount"), "{missing:?}");

        // Editing keeps the movement and changes what the form changed.
        let form = finance_form(app.clone(), FormPayload { context: context.clone(), kind: "movement".into(), id: Some(expense.clone()), prefill: Map::new() }).expect("form");
        assert_eq!(form.kind, "expense");
        assert_eq!(form.fields.iter().find(|field| field.key == "amount").map(|field| field.value.clone()), Some(json!("1500,50")));
        saved(apply(&app, &context, "expense", Some(&expense), json!({ "amount": "1500,50", "accountId": bank, "categoryId": food, "description": "Coto Palermo" }), None));
        let edited = crate::finance::finance_get_transaction(app.clone(), crate::finance::GetFinanceRecordPayload { context: context.clone(), id: expense.clone() }).expect("edited");
        assert_eq!(edited.description, "Coto Palermo");

        // With a card it waits for its statement, and dollars are accepted.
        let unpaid = apply(&app, &context, "expense", None, json!({ "amount": "20", "currency": "USD", "accountId": card, "description": "Spotify" }), None);
        assert!(unpaid.ok && unpaid.message.contains("a pagar"), "{unpaid:?}");
        let wrong = apply(&app, &context, "expense", None, json!({ "amount": "20", "currency": "USD", "accountId": bank, "description": "Dólares en pesos" }), None);
        assert!(!wrong.ok && wrong.errors.contains_key("currency"), "{wrong:?}");

        // Another income counts in the balance of the month.
        saved(apply(&app, &context, "income", None, json!({ "amount": "5000", "accountId": bank, "description": "Venta de la bici" }), None));
        let view = overview(&app, &context, &month);
        let balance = &view["balance"]["rows"][0];
        assert_eq!((balance["otherIncome"].as_str(), balance["expenses"].as_str(), balance["left"].as_str()), (Some("5000.00"), Some("1500.50"), Some("3499.50")), "{balance}");
        assert_eq!(view["balance"]["otherIncomes"][0]["description"], "Venta de la bici");

        // A loan: its installment is an expense of «Préstamos» and can be taken back.
        let loan = saved(apply(&app, &context, "loan", None, json!({ "name": "Auto", "installmentCount": "3", "installmentAmount": "1000", "firstDueDate": today(), "accountId": bank }), None));
        let paid = apply(&app, &context, "loan-payment", None, json!({ "loanId": loan, "amount": "1.100" }), None);
        assert!(paid.ok && paid.message.contains("1 de 3"), "{paid:?}");
        let view = overview(&app, &context, &month);
        assert_eq!(view["loans"]["rows"][0]["status"], "paid");
        assert_eq!(view["loans"]["paid"][0], json!(["ARS", "1100.00"]));
        let payment = paid.saved_id.expect("payment");
        let owned = crate::finance::finance_delete_transaction(app.clone(), crate::finance::DeleteFinanceEntityPayload { context: context.clone(), id: payment.clone() });
        assert!(owned.is_err_and(|error| error.message.contains("préstamo")), "a loan's payment is removed from the loan");
        let unpay = finance_apply(app.clone(), ApplyPayload { context: context.clone(), kind: "loan".into(), id: Some(loan.clone()), action: Some("unpay-last".into()), values: Map::new(), request_id: None }).expect("unpay");
        assert!(unpay.ok && unpay.message.contains("0 pagadas"), "{unpay:?}");

        // A budget against what its category spent.
        saved(apply(&app, &context, "budget", None, json!({ "categoryId": food, "currency": "ARS", "amount": "1000" }), None));
        let view = overview(&app, &context, &month);
        assert_eq!(view["budgets"][0]["status"], "over");
        assert_eq!(view["budgets"][0]["spent"], "1500.50");
        let twice = apply(&app, &context, "budget", None, json!({ "categoryId": food, "currency": "ARS", "amount": "2000" }), None);
        assert!(!twice.ok && twice.message.contains("ya tiene un presupuesto"), "{twice:?}");

        // Documents: a ticket with its products, a salary with only its net, a statement.
        let ticket = apply(&app, &context, "purchase", None, json!({ "merchantName": "Coto", "accountId": bank, "items": [{ "originalDescription": "Leche", "quantity": "2", "unitPrice": "600" }] }), None);
        assert!(ticket.ok, "{ticket:?}");
        let salary = apply(&app, &context, "salary", None, json!({ "employer": "ACME", "period": month, "paymentDate": today(), "accountId": bank, "netAmount": "900000" }), None);
        assert!(salary.ok, "{salary:?}");
        let statement = apply(
            &app,
            &context,
            "card-statement",
            None,
            json!({
                "accountId": card, "currency": "ARS", "issuer": "Galicia", "period": month, "closingDate": today(), "dueDate": today(),
                "totalDue": "3000", "items": [{ "purchaseDate": today(), "description": "Farmacity", "amount": "3000", "itemType": "purchase" }]
            }),
            None,
        );
        assert!(statement.ok, "{statement:?}");
        let services = saved(apply(&app, &context, "service", None, json!({ "name": "Internet", "categoryId": food, "currency": "ARS", "expectedAmount": "15000" }), None));
        let service_paid = apply(&app, &context, "service-payment", None, json!({ "serviceId": services, "accountId": bank }), None);
        assert!(service_paid.ok && service_paid.message.contains("15.000"), "the expected amount when none is typed: {service_paid:?}");

        // The whole history is searched and exported.
        let search = serde_json::from_value(json!({ "context": context, "search": "coto" })).expect("search");
        let found = serde_json::to_value(crate::finance_screen::finance_search(app.clone(), search).expect("found")).expect("json");
        assert!(found["count"].as_u64().unwrap_or_default() >= 2, "{found}");
        let export = serde_json::from_value(json!({ "context": context, "format": "xlsx" })).expect("export");
        let file = crate::finance_export::finance_export(app.clone(), export).expect("file");
        assert!(file.rows >= 5 && file.file_name == "finanzas-historial.xlsx", "{} {}", file.rows, file.file_name);

        // What can be edited, section by section.
        let sections = finance_manage(app.clone(), context.clone()).expect("manage");
        let rows = |key: &str| sections.iter().find(|section| section.key == key).map(|section| section.rows.len()).unwrap_or_default();
        assert_eq!((rows("accounts"), rows("loans"), rows("budgets"), rows("services")), (2, 1, 1, 1));

        // And deleting.
        let deleted = finance_apply(app.clone(), ApplyPayload { context: context.clone(), kind: "movement".into(), id: Some(expense.clone()), action: Some("delete".into()), values: Map::new(), request_id: None }).expect("delete");
        assert!(deleted.ok, "{deleted:?}");
        assert!(crate::finance::finance_get_transaction(app.clone(), crate::finance::GetFinanceRecordPayload { context: context.clone(), id: expense }).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_request_id_is_short_and_plain() {
        assert!(valid_request_id("0b8f6d3e-6f7a-4c1e-9f1a-2b3c4d5e6f70"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("a b"));
        assert!(!valid_request_id(&"x".repeat(MAX_REQUEST_ID_CHARS + 1)));
    }

    #[test]
    fn a_line_total_is_quantity_by_price_to_the_cent() {
        assert_eq!(line_total("3", "1200.5").as_deref(), Some("3601.50"));
        assert_eq!(line_total("0.333", "100").as_deref(), Some("33.30"));
    }

    #[test]
    fn the_app_keeps_its_origin_and_the_screen_loads_by_hand() {
        let context = |source: &str| FinanceContext { library_path: "/b".into(), android_directory_uri: None, actor_library_user_id: OWNER.into(), source: source.into() };
        assert_eq!(manual(&context("android")).source, "android");
        assert_eq!(manual(&context("app")).source, "manual");
        assert_eq!(manual(&context("telegram")).source, "manual");
    }

    #[test]
    fn a_builder_error_speaks_to_the_person() {
        let catalog = Catalog { today: "2026-11-18".into(), ..Catalog::default() };
        let form = forms::build("expense", &catalog, &Editing::default(), &Values::new()).expect("form");
        let error = AgentInputError { error: "invalid-finance-transaction", invalid_fields: vec!["amount".into(), "effectiveDate".into()], instruction: "para el modelo".into() };
        let result = input_error(&form, &error);
        assert!(!result.ok);
        assert_eq!(result.errors["amount"], "Revisá «Importe».");
        assert_eq!(result.errors["effectiveDate"], "Revisá «Fecha».");
        assert!(!result.message.contains("modelo"));
        let currency = AgentInputError { error: "finance-account-currency-mismatch", invalid_fields: vec!["currency".into()], instruction: String::new() };
        assert_eq!(input_error(&form, &currency).message, "La moneda tiene que ser la de la cuenta.");
    }
}
