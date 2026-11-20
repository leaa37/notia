//! Finanzas: el núcleo que comparten la pantalla, la app de Android y el
//! asistente.
//!
//! El modelo es deliberadamente simple (esquema 36):
//! - **Movimientos**: gastos e ingresos con nombre, categoría, importe,
//!   moneda, fecha y, si se quiere, una cuenta. El sueldo es un ingreso de la
//!   categoría «Sueldo».
//! - **Pagos**: pagar un servicio, una cuota de tarjeta o una cuota de un
//!   préstamo crea el gasto del mes en que se paga; lo pendiente se calcula.
//!   Un servicio es solo su nombre y una cuota de préstamo no tiene importe
//!   hasta que se paga: nada pendiente se muestra con un importe inventado.
//! - **Tarjetas**: cada compra lleva su total y sus cuotas; el resumen de
//!   cada mes se arma solo con las cuotas que vencen.
//! - **Ahorro**: reservas con depósitos y retiros, y compra o venta de
//!   dólares.
//!
//! Este módulo tiene el contexto, los errores, los importes, los meses, las
//! categorías que crea Notia y las herramientas de la pestaña Dev.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, types::ValueRef, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::database::open_library_connection;

// ---------------------------------------------------------------------------
// Errors and context
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceCommandError {
    /// `validation`, `conflict`, `notFound` or `storage`.
    pub code: &'static str,
    pub message: String,
}

impl FinanceCommandError {
    /// Something the person can fix in what they loaded.
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self { code: "validation", message: message.into() }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self { code: "notFound", message: message.into() }
    }

    pub(crate) fn storage(message: impl Into<String>) -> Self {
        Self { code: "storage", message: message.into() }
    }
}

/// A plain message is a failure of storage or permissions.
impl From<String> for FinanceCommandError {
    fn from(message: String) -> Self {
        Self::storage(message)
    }
}

impl From<&str> for FinanceCommandError {
    fn from(message: &str) -> Self {
        Self::storage(message)
    }
}

impl From<rusqlite::Error> for FinanceCommandError {
    fn from(error: rusqlite::Error) -> Self {
        Self::storage(storage_message(&error))
    }
}

pub type FinanceCommandResult<T> = Result<T, FinanceCommandError>;

pub(crate) fn storage_message(error: &rusqlite::Error) -> String {
    format!("No se pudo usar la base de Finanzas: {error}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceContext {
    pub library_path: String,
    pub android_directory_uri: Option<String>,
    pub actor_library_user_id: String,
    /// `app`, `manual` (forms of the screen), `android` (the Notia Finanzas
    /// app), `telegram` or `public-url`.
    pub source: String,
}

pub(crate) fn validate_context(context: &FinanceContext, app: &crate::host::AppHandle) -> Result<Connection, String> {
    #[cfg(target_os = "android")]
    {
        let directory_uri = context
            .android_directory_uri
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "La biblioteca perdió su URI SAF. Volvé a seleccionarla.".to_string())?;
        let connection = crate::database::open_mobile_library_connection(app, directory_uri)?;
        validate_finance_actor(&connection, context)?;
        Ok(connection)
    }
    #[cfg(target_os = "ios")]
    {
        let _ = (context, app);
        Err("Finanzas todavía no está disponible en iOS.".to_string())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        if context.library_path.trim().is_empty() {
            return Err("La librería es obligatoria.".to_string());
        }
        let connection = open_library_connection(&context.library_path)?;
        validate_finance_actor(&connection, context)?;
        Ok(connection)
    }
}

fn validate_finance_actor(connection: &Connection, context: &FinanceContext) -> Result<(), String> {
    if !matches!(context.source.as_str(), "app" | "public-url" | "telegram" | "manual" | "android") {
        return Err("El origen de la operación financiera no es válido.".to_string());
    }
    let actor_id = context.actor_library_user_id.trim();
    if actor_id.is_empty() {
        return Err("El usuario de la biblioteca es obligatorio para acceder a Finanzas.".to_string());
    }
    let role_id: Option<String> = connection
        .query_row("SELECT role_id FROM library_users WHERE id=?1", [actor_id], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(role_id) = role_id else {
        return Err("El usuario de la biblioteca no está autorizado para Finanzas.".to_string());
    };
    if role_id != "role-owner" {
        let has_confidential_context: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM library_user_contexts WHERE user_id=?1 AND lower(context_tag)=lower('#Confidencial'))",
                [actor_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !has_confidential_context {
            return Err("El usuario no tiene el contexto #Confidencial requerido para Finanzas.".to_string());
        }
    }
    Ok(())
}

/// Event the Finanzas screen listens to, to read its figures again.
pub const FINANCE_DATA_CHANGED_EVENT: &str = "notia:finance-data-changed";

/// Every finance write ends here: it saves the Android copy back to the
/// library, tells the open screen that the records changed and checks the
/// budgets of the month.
pub(crate) fn sync_context(context: &FinanceContext, app: &crate::host::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let directory_uri = context
            .android_directory_uri
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "La biblioteca perdió su URI SAF. Volvé a seleccionarla.".to_string())?;
        crate::database::sync_mobile_library_connection(app, directory_uri)?;
    }
    use crate::host::Emitter;
    if let Err(error) = app.emit(FINANCE_DATA_CHANGED_EVENT, ()) {
        log::warn!("[notia:finance] evento de cambio no emitido: {error}");
    }
    crate::finance_budgets::check_alerts_later(app, context);
    Ok(())
}

/// Runs `write` in one transaction of the library and, once committed,
/// syncs it. An error rolls everything back.
pub(crate) fn write<T>(
    app: &crate::host::AppHandle,
    context: &FinanceContext,
    write: impl FnOnce(&Connection) -> FinanceCommandResult<T>,
) -> FinanceCommandResult<T> {
    let mut connection = validate_context(context, app)?;
    let transaction = connection.transaction()?;
    let result = write(&transaction)?;
    transaction.commit()?;
    drop(connection);
    sync_context(context, app)?;
    Ok(result)
}

/// Reads with a connection of the library.
pub(crate) fn read<T>(
    app: &crate::host::AppHandle,
    context: &FinanceContext,
    read: impl FnOnce(&Connection) -> FinanceCommandResult<T>,
) -> FinanceCommandResult<T> {
    let connection = validate_context(context, app)?;
    read(&connection)
}

// ---------------------------------------------------------------------------
// Time, ids
// ---------------------------------------------------------------------------

/// Seconds since the epoch, as the other modules stamp their rows.
pub(crate) fn now() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_else(|_| "0".to_string())
}

pub(crate) fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Amounts
// ---------------------------------------------------------------------------

pub(crate) const CURRENCIES: [&str; 2] = ["ARS", "USD"];

pub(crate) fn valid_currency(value: &str) -> bool {
    CURRENCIES.contains(&value)
}

/// Cents of a stored amount (`1500.5`, `1500.50`, `1500`).
pub(crate) fn parse_cents(value: &str) -> Result<i128, ()> {
    let trimmed = value.trim();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    if digits.is_empty() || digits.len() > 32 {
        return Err(());
    }
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err(());
    }
    let cents = format!("{fraction:0<2}").parse::<i128>().map_err(|_| ())?;
    let value = whole.parse::<i128>().map_err(|_| ())?.checked_mul(100).and_then(|value| value.checked_add(cents)).ok_or(())?;
    Ok(if negative { -value } else { value })
}

/// Cents as stored: `1500.50`.
pub(crate) fn format_cents(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let absolute = value.abs();
    format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
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

/// Cents of an amount as people type it or a tool sends it: `1500`,
/// `1.500`, `1.500,50`, `1500,5`, `1500.50`, `$ 1.500` or a JSON number.
/// Negative or empty amounts are `None`.
pub(crate) fn typed_cents(value: &Value) -> Option<i128> {
    let raw = match value {
        Value::Number(number) => {
            let text = number.to_string();
            if text.starts_with('-') || text.contains(['e', 'E']) {
                return None;
            }
            text
        }
        Value::String(text) => text.trim().to_string(),
        _ => return None,
    };
    let mut compact = raw;
    for symbol in ["US$", "u$s", "U$S", "ARS", "ars", "USD", "usd", "$"] {
        compact = compact.replace(symbol, "");
    }
    let compact = compact.split_whitespace().collect::<String>();
    if compact.is_empty() || compact.chars().any(|c| !(c.is_ascii_digit() || c == '.' || c == ',')) {
        return None;
    }
    let comma = compact.rfind(',');
    let dot = compact.rfind('.');
    let (mut whole, mut fraction) = (compact.as_str(), "");
    if let Some(index) = comma.max(dot) {
        let separator = compact.as_bytes()[index] as char;
        let trailing = compact.len() - index - 1;
        let repeated = compact.chars().filter(|c| *c == separator).count() > 1;
        let both = comma.is_some() && dot.is_some();
        // «1.500» is fifteen hundred; «1.5» and «1500,50» carry cents.
        if trailing > 0 && trailing <= 2 && (both || !repeated) {
            whole = &compact[..index];
            fraction = &compact[index + 1..];
        }
    }
    let whole = whole.replace(['.', ','], "");
    let whole = if whole.is_empty() { "0".to_string() } else { whole };
    if whole.len() > 15 || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    parse_cents(&format!("{whole}.{fraction}")).ok().or_else(|| parse_cents(&whole).ok())
}

/// A stored amount as it is typed in Argentina: `1500,50`, or `1500` when
/// it has no cents.
pub(crate) fn amount_input(cents: i128) -> String {
    if cents % 100 == 0 {
        (cents / 100).to_string()
    } else {
        format!("{},{:02}", cents / 100, (cents % 100).abs())
    }
}

/// Splits `total` in `count` installments: the same amount each, and the
/// cents that do not divide go to the first one.
pub(crate) fn installment_amounts(total: i128, count: u32) -> Vec<i128> {
    if count == 0 {
        return Vec::new();
    }
    let count_i = i128::from(count);
    let base = total / count_i;
    let rest = total - base * count_i;
    (0..count).map(|index| if index == 0 { base + rest } else { base }).collect()
}

// ---------------------------------------------------------------------------
// Dates and months
// ---------------------------------------------------------------------------

pub(crate) fn valid_iso_date(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok() && value.len() == 10
}

pub(crate) fn valid_month(value: &str) -> bool {
    value.len() == 7 && chrono::NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").is_ok()
}

/// `month` moved `offset` months (`2026-11` + 2 = `2027-01`).
pub(crate) fn add_months(month: &str, offset: i32) -> String {
    let year = month.get(0..4).and_then(|value| value.parse::<i32>().ok()).unwrap_or(1970);
    let number = month.get(5..7).and_then(|value| value.parse::<i32>().ok()).unwrap_or(1);
    let index = year * 12 + (number - 1) + offset;
    format!("{:04}-{:02}", index.div_euclid(12), index.rem_euclid(12) + 1)
}

/// Months from `from` to `to` (`2026-11` to `2027-01` is 2).
pub(crate) fn months_between(from: &str, to: &str) -> i32 {
    let index = |month: &str| {
        let year = month.get(0..4).and_then(|value| value.parse::<i32>().ok()).unwrap_or(1970);
        let number = month.get(5..7).and_then(|value| value.parse::<i32>().ok()).unwrap_or(1);
        year * 12 + number - 1
    };
    index(to) - index(from)
}

/// First and last day of `month`.
pub(crate) fn month_range(month: &str) -> Option<(String, String)> {
    let first = chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok()?;
    let next = chrono::NaiveDate::parse_from_str(&format!("{}-01", add_months(month, 1)), "%Y-%m-%d").ok()?;
    let last = next.pred_opt()?;
    Some((first.format("%Y-%m-%d").to_string(), last.format("%Y-%m-%d").to_string()))
}

pub(crate) fn days_in_month(month: &str) -> u32 {
    month_range(month).and_then(|(_, last)| last.get(8..10).and_then(|day| day.parse().ok())).unwrap_or(30)
}

const MONTH_NAMES: [&str; 12] =
    ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"];

/// «noviembre 2026».
pub(crate) fn month_label(month: &str) -> String {
    let name = month.get(5..7).and_then(|value| value.parse::<usize>().ok()).and_then(|value| MONTH_NAMES.get(value.wrapping_sub(1)));
    match (name, month.get(0..4)) {
        (Some(name), Some(year)) => format!("{name} {year}"),
        _ => month.to_string(),
    }
}

/// «nov 2026».
pub(crate) fn month_short(month: &str) -> String {
    let name = month.get(5..7).and_then(|value| value.parse::<usize>().ok()).and_then(|value| MONTH_NAMES.get(value.wrapping_sub(1)));
    match (name, month.get(2..4)) {
        (Some(name), Some(year)) => format!("{} {year}", &name[..3]),
        _ => month.to_string(),
    }
}

/// «20/11».
pub(crate) fn day_label(date: &str) -> String {
    match (date.get(8..10), date.get(5..7)) {
        (Some(day), Some(month)) => format!("{day}/{month}"),
        _ => date.to_string(),
    }
}

// ---------------------------------------------------------------------------
// What Notia creates
// ---------------------------------------------------------------------------

/// The category of every service payment, created again if it was deleted.
pub(crate) const CATEGORY_SERVICES: (&str, &str) = ("category-services", "Servicios");
/// The category of every loan installment.
pub(crate) const CATEGORY_LOANS: (&str, &str) = ("category-loans", "Préstamos");
/// The income category of the salary.
pub(crate) const CATEGORY_SALARY: (&str, &str) = ("category-salary", "Sueldo");
pub(crate) const CATEGORY_OTHER: (&str, &str) = ("category-other", "Otros");
pub(crate) const CATEGORY_OTHER_INCOME: (&str, &str) = ("category-other-income", "Otros ingresos");

const DEFAULT_EXPENSE_CATEGORIES: [(&str, &str); 12] = [
    ("category-supermarket", "Supermercado"),
    ("category-food", "Comida y delivery"),
    ("category-transport", "Transporte"),
    ("category-home", "Hogar"),
    ("category-health", "Salud"),
    ("category-leisure", "Salidas y ocio"),
    ("category-clothing", "Ropa"),
    ("category-education", "Educación"),
    ("category-taxes", "Impuestos"),
    CATEGORY_SERVICES,
    CATEGORY_LOANS,
    CATEGORY_OTHER,
];
const DEFAULT_INCOME_CATEGORIES: [(&str, &str); 2] = [CATEGORY_SALARY, CATEGORY_OTHER_INCOME];
const DEFAULT_RESERVES: [(&str, &str, &str); 2] =
    [("reserve-pesos", "Ahorro en pesos", "ARS"), ("reserve-dollars", "Ahorro en dólares", "USD")];

/// The categories and reserves a new Finanzas starts with.
pub(crate) fn seed_defaults(connection: &Connection) -> rusqlite::Result<()> {
    let stamp = now();
    for (kind, categories) in [("expense", &DEFAULT_EXPENSE_CATEGORIES[..]), ("income", &DEFAULT_INCOME_CATEGORIES[..])] {
        for (id, name) in categories {
            connection.execute(
                "INSERT OR IGNORE INTO finance_categories (id,name,kind,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
                params![id, name, kind, stamp],
            )?;
        }
    }
    for (id, name, currency) in DEFAULT_RESERVES {
        connection.execute(
            "INSERT OR IGNORE INTO finance_reserves (id,name,currency,goal,created_at,updated_at) VALUES (?1,?2,?3,NULL,?4,?4)",
            params![id, name, currency, stamp],
        )?;
    }
    Ok(())
}

/// The id of a category Notia uses on its own, created again (with its
/// name) when the person deleted it.
pub(crate) fn ensure_category(connection: &Connection, (id, name): (&str, &str), kind: &str) -> rusqlite::Result<String> {
    let stamp = now();
    connection.execute(
        "INSERT OR IGNORE INTO finance_categories (id,name,kind,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
        params![id, name, kind, stamp],
    )?;
    Ok(id.to_string())
}

// ---------------------------------------------------------------------------
// Dev
// ---------------------------------------------------------------------------

/// Every table of Finanzas, in the order they are emptied.
pub(crate) const FINANCE_TABLES: &[&str] = &[
    "finance_applied_requests",
    "finance_budget_alerts",
    "finance_budgets",
    "finance_savings_movements",
    "finance_reserves",
    "finance_movements",
    "finance_card_purchases",
    "finance_cards",
    "finance_loans",
    "finance_services",
    "finance_accounts",
    "finance_categories",
];
const DEV_PAGE_SIZE: u32 = 50;
const DEV_MAX_PAGE_SIZE: u32 = 200;
const DEV_MAX_SQL_CHARS: usize = 20_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDevTable {
    pub name: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDevQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub total_rows: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDevTableQueryPayload {
    pub context: FinanceContext,
    pub table_name: String,
    pub page: u32,
    pub page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDevSqlQueryPayload {
    pub context: FinanceContext,
    pub sql: String,
    pub page: u32,
    pub page_size: Option<u32>,
}

fn dev_page_size(value: Option<u32>) -> u32 {
    value.unwrap_or(DEV_PAGE_SIZE).clamp(1, DEV_MAX_PAGE_SIZE)
}

fn validate_dev_sql(sql: &str) -> Result<String, String> {
    let normalized = sql.trim().trim_end_matches(';').trim();
    if normalized.is_empty() || normalized.len() > DEV_MAX_SQL_CHARS {
        return Err("La consulta SQL debe tener entre 1 y 20000 caracteres.".to_string());
    }
    if normalized.contains(';') {
        return Err("La consola Dev admite una única consulta SQL.".to_string());
    }
    let lower = normalized.to_ascii_lowercase();
    if !(lower.starts_with("select") || lower.starts_with("with")) {
        return Err("La consola Dev solo admite consultas SELECT o WITH de solo lectura.".to_string());
    }
    for keyword in ["insert", "update", "delete", "replace", "drop", "alter", "create", "attach", "detach", "vacuum", "pragma"] {
        if lower.split(|character: char| !character.is_ascii_alphanumeric() && character != '_').any(|token| token == keyword) {
            return Err("La consola Dev no admite sentencias que modifiquen la base de datos.".to_string());
        }
    }
    Ok(normalized.to_string())
}

fn dev_value(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(value) => Some(value.to_string()),
        ValueRef::Real(value) => Some(value.to_string()),
        ValueRef::Text(value) => Some(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Some(format!("<blob: {} bytes>", value.len())),
    }
}

fn dev_query(connection: &Connection, sql: &str, page: u32, page_size: u32) -> Result<FinanceDevQueryResult, String> {
    let total_rows: i64 = connection
        .query_row(&format!("SELECT COUNT(*) FROM ({sql}) AS finance_dev_count"), [], |row| row.get(0))
        .map_err(|_| "No se pudo contar el resultado de la consulta SQL.".to_string())?;
    let offset = i64::from(page) * i64::from(page_size);
    let mut statement = connection
        .prepare(&format!("SELECT * FROM ({sql}) AS finance_dev_result LIMIT ?1 OFFSET ?2"))
        .map_err(|_| "La consulta SQL no es válida.".to_string())?;
    let columns = statement.column_names().iter().map(|name| (*name).to_string()).collect();
    let rows = statement
        .query_map(params![page_size, offset], |row| {
            (0..row.as_ref().column_count()).map(|index| row.get_ref(index).map(dev_value)).collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| "No se pudo ejecutar la consulta SQL.".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "No se pudo leer el resultado de la consulta SQL.".to_string())?;
    Ok(FinanceDevQueryResult { columns, rows, total_rows, page, page_size })
}

pub fn finance_dev_list_tables() -> Vec<FinanceDevTable> {
    let mut names = FINANCE_TABLES.to_vec();
    names.sort_unstable();
    names.into_iter().map(|name| FinanceDevTable { name }).collect()
}

pub fn finance_dev_query_table(app: crate::host::AppHandle, payload: FinanceDevTableQueryPayload) -> FinanceCommandResult<FinanceDevQueryResult> {
    if !FINANCE_TABLES.contains(&payload.table_name.as_str()) {
        return Err(FinanceCommandError::not_found("La tabla de Finanzas no existe."));
    }
    let connection = validate_context(&payload.context, &app)?;
    dev_query(&connection, &format!("SELECT * FROM {}", payload.table_name), payload.page, dev_page_size(payload.page_size))
        .map_err(FinanceCommandError::invalid)
}

pub fn finance_dev_query_sql(app: crate::host::AppHandle, payload: FinanceDevSqlQueryPayload) -> FinanceCommandResult<FinanceDevQueryResult> {
    let sql = validate_dev_sql(&payload.sql).map_err(FinanceCommandError::invalid)?;
    let connection = validate_context(&payload.context, &app)?;
    dev_query(&connection, &sql, payload.page, dev_page_size(payload.page_size)).map_err(FinanceCommandError::invalid)
}

/// Empties Finanzas and leaves the default categories and reserves.
pub(crate) fn clear_finance_data(connection: &Connection) -> rusqlite::Result<()> {
    for table in FINANCE_TABLES {
        connection.execute(&format!("DELETE FROM {table}"), [])?;
    }
    seed_defaults(connection)
}

pub fn finance_clear_all_data(app: crate::host::AppHandle, context: FinanceContext) -> FinanceCommandResult<()> {
    write(&app, &context, |connection| Ok(clear_finance_data(connection)?))
}

/// A month of example data, to try the screen: accounts, a card with
/// purchases in installments, services, a loan, a budget and savings.
pub(crate) fn seed_demo_data(connection: &Connection) -> rusqlite::Result<()> {
    seed_defaults(connection)?;
    let month = current_month();
    let previous = add_months(&month, -1);
    let next = add_months(&month, 1);
    let stamp = now();
    let date = |month: &str, day: u32| format!("{month}-{day:02}");
    connection.execute_batch(&format!(
        "INSERT OR IGNORE INTO finance_accounts (id,name,created_at,updated_at) VALUES
            ('dev-account-bank','Banco demo','{stamp}','{stamp}'),
            ('dev-account-cash','Efectivo demo','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_cards (id,name,created_at,updated_at) VALUES
            ('dev-card-visa','Visa demo','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_services (id,name,created_at,updated_at) VALUES
            ('dev-service-internet','Internet demo','{stamp}','{stamp}'),
            ('dev-service-power','Luz demo','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_loans (id,name,currency,amount,installment_count,first_month,created_at,updated_at) VALUES
            ('dev-loan-bank','Préstamo demo','ARS','600000.00',12,'{previous}','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_card_purchases (id,card_id,name,category_id,currency,total,installment_count,first_month,created_at,updated_at) VALUES
            ('dev-purchase-fridge','dev-card-visa','Heladera demo','category-home','ARS','720000.00',12,'{previous}','{stamp}','{stamp}'),
            ('dev-purchase-shoes','dev-card-visa','Zapatillas demo','category-clothing','ARS','90000.00',3,'{month}','{stamp}','{stamp}'),
            ('dev-purchase-app','dev-card-visa','Suscripción demo','category-leisure','USD','12.00',1,'{next}','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_budgets (id,category_id,currency,amount,created_at,updated_at) VALUES
            ('dev-budget-market','category-supermarket','ARS','300000.00','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_movements (id,kind,name,category_id,account_id,amount,currency,date,service_id,service_month,card_purchase_id,loan_id,installment_number,source,created_at,updated_at) VALUES
            ('dev-move-salary','income','Sueldo','category-salary','dev-account-bank','1500000.00','ARS','{d1}',NULL,NULL,NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-salary-prev','income','Sueldo','category-salary','dev-account-bank','1450000.00','ARS','{p1}',NULL,NULL,NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-market','expense','Supermercado demo','category-supermarket','dev-account-bank','185000.00','ARS','{d5}',NULL,NULL,NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-market-prev','expense','Supermercado demo','category-supermarket','dev-account-bank','210000.00','ARS','{p5}',NULL,NULL,NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-fuel','expense','Nafta demo','category-transport','dev-account-cash','45000.00','ARS','{d3}',NULL,NULL,NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-internet','expense','Internet demo','category-services','dev-account-bank','25500.00','ARS','{d1}','dev-service-internet','{month}',NULL,NULL,NULL,'app','{stamp}','{stamp}'),
            ('dev-move-fridge-1','expense','Heladera demo · cuota 1/12','category-home','dev-account-bank','60000.00','ARS','{p1}',NULL,NULL,'dev-purchase-fridge',NULL,1,'app','{stamp}','{stamp}'),
            ('dev-move-loan-1','expense','Préstamo demo · cuota 1/12','category-loans','dev-account-bank','62000.00','ARS','{p1}',NULL,NULL,NULL,'dev-loan-bank',1,'app','{stamp}','{stamp}');
         INSERT OR IGNORE INTO finance_savings_movements (id,reserve_id,kind,amount,pesos,date,note,source,created_at,updated_at) VALUES
            ('dev-saving-pesos','reserve-pesos','deposit','100000.00',NULL,'{d1}','','app','{stamp}','{stamp}'),
            ('dev-saving-dollars','reserve-dollars','deposit','100.00','120000.00','{d3}','','app','{stamp}','{stamp}');",
        d1 = date(&month, 1),
        d3 = date(&month, 3),
        d5 = date(&month, 5),
        p1 = date(&previous, 1),
        p5 = date(&previous, 5),
    ))
}

pub fn finance_dev_seed_demo_data(app: crate::host::AppHandle, context: FinanceContext) -> FinanceCommandResult<()> {
    write(&app, &context, |connection| Ok(seed_demo_data(connection)?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A migrated database in memory, with the Owner.
    pub(crate) fn database() -> Connection {
        let connection = Connection::open_in_memory().expect("memory");
        crate::database::migrate(&connection).expect("migrate");
        connection
    }

    #[test]
    fn amounts_are_read_as_people_type_them() {
        let cents = |text: &str| typed_cents(&Value::String(text.into()));
        assert_eq!(cents("1500"), Some(150_000));
        assert_eq!(cents("1.500"), Some(150_000));
        assert_eq!(cents("1.500,50"), Some(150_050));
        assert_eq!(cents("1500,5"), Some(150_050));
        assert_eq!(cents("1500.50"), Some(150_050));
        assert_eq!(cents("$ 1.234.567"), Some(123_456_700));
        assert_eq!(cents("US$ 12,99"), Some(1_299));
        assert_eq!(cents("-5"), None);
        assert_eq!(cents("abc"), None);
        assert_eq!(cents(""), None);
        assert_eq!(typed_cents(&serde_json::json!(1500.5)), Some(150_050));
        assert_eq!(amount_input(150_050), "1500,50");
        assert_eq!(amount_input(150_000), "1500");
    }

    #[test]
    fn amounts_read_as_in_argentina() {
        assert_eq!(money_text(8_500_000, "ARS"), "$ 85.000");
        assert_eq!(money_text(120_050, "USD"), "US$ 1.200,50");
        assert_eq!(money_text(-150_000, "ARS"), "-$ 1.500");
        assert_eq!(money_text(99, "ARS"), "$ 0,99");
    }

    #[test]
    fn installments_add_up_to_the_total() {
        assert_eq!(installment_amounts(10_000, 3), vec![3_334, 3_333, 3_333]);
        assert_eq!(installment_amounts(10_000, 3).iter().sum::<i128>(), 10_000);
        assert_eq!(installment_amounts(5_000, 1), vec![5_000]);
    }

    #[test]
    fn months_move_across_years() {
        assert_eq!(add_months("2026-11", 2), "2027-01");
        assert_eq!(add_months("2026-01", -1), "2025-12");
        assert_eq!(months_between("2026-11", "2027-01"), 2);
        assert_eq!(month_range("2028-02"), Some(("2028-02-01".into(), "2028-02-29".into())));
        assert_eq!(month_label("2026-11"), "noviembre 2026");
        assert_eq!(month_short("2026-11"), "nov 26");
        assert!(valid_month("2026-12") && !valid_month("2026-13") && !valid_month("2026-1"));
        assert!(valid_iso_date("2026-02-28") && !valid_iso_date("2026-02-30"));
    }

    #[test]
    fn a_new_finanzas_starts_with_its_categories_and_reserves() {
        let connection = database();
        let count = |sql: &str| connection.query_row(sql, [], |row| row.get::<_, i64>(0)).expect("count");
        assert_eq!(count("SELECT COUNT(*) FROM finance_categories WHERE kind='expense'"), 12);
        assert_eq!(count("SELECT COUNT(*) FROM finance_categories WHERE kind='income'"), 2);
        assert_eq!(count("SELECT COUNT(*) FROM finance_reserves"), 2);
        seed_demo_data(&connection).expect("demo");
        clear_finance_data(&connection).expect("clear");
        assert_eq!(count("SELECT COUNT(*) FROM finance_movements"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM finance_categories"), 14);
    }

    #[test]
    fn the_dev_console_only_reads() {
        assert!(validate_dev_sql("SELECT * FROM finance_movements").is_ok());
        assert!(validate_dev_sql("DELETE FROM finance_movements").is_err());
        assert!(validate_dev_sql("SELECT 1; SELECT 2").is_err());
        assert_eq!(finance_dev_list_tables().len(), FINANCE_TABLES.len());
    }
}
