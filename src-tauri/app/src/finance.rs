use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, types::ValueRef, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::finance_matching::{LinkOutcome, CARD_UNPAID};
use crate::finance_reconciliation::normalize_service_text;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceCommandError {
    pub code: &'static str,
    pub message: String,
}

impl From<String> for FinanceCommandError {
    fn from(message: String) -> Self {
        let lower = message.to_lowercase();
        let code = if lower.contains("no existe") || lower.contains("no est") {
            "notFound"
        } else if lower.contains("duplic")
            || lower.contains("ya existe")
            || lower.contains("ya fue registrado")
            || lower.contains("registrado anteriormente")
        {
            "conflict"
        } else if lower.contains("requiere")
            || lower.contains("inválid")
            || lower.contains("debe")
            || lower.contains("diferencia")
            || lower.contains("no coincide")
            || lower.contains("suma de líneas")
            || lower.contains("importe")
        {
            "validation"
        } else {
            "storage"
        };
        Self { code, message }
    }
}

impl FinanceCommandError {
    /// Something the person can fix in what they loaded.
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self { code: "validation", message: message.into() }
    }
}

impl From<&str> for FinanceCommandError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

pub type FinanceCommandResult<T> = Result<T, FinanceCommandError>;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::database::open_library_connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceContext {
    pub library_path: String,
    pub android_directory_uri: Option<String>,
    pub actor_library_user_id: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceAccount {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub currency: String,
    pub active: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceCategory {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub parent_id: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceTransaction {
    pub id: String,
    pub transaction_type: String,
    pub amount: String,
    pub currency: String,
    pub effective_date: String,
    pub account_id: String,
    pub destination_account_id: Option<String>,
    pub category_id: Option<String>,
    pub description: String,
    pub source: String,
    pub status: String,
    pub actor_user_id: Option<i64>,
    #[serde(default)]
    pub actor_library_user_id: Option<String>,
    pub source_artifact_id: Option<String>,
    pub service_id: Option<String>,
    pub merchant_id: Option<String>,
    pub operation_fingerprint: Option<String>,
    pub installment_id: Option<String>,
    pub source_reference: Option<String>,
    pub raw_source: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Day of the purchase when it is not the day the expense counts: card
    /// expenses count in the month their statement is paid.
    #[serde(default)]
    pub purchase_date: Option<String>,
}

const TRANSACTION_SELECT: &str = "SELECT t.id,t.transaction_type,t.amount,t.currency,t.effective_date,
        t.account_id,t.destination_account_id,t.category_id,t.description,t.source,t.status,
        t.actor_user_id,t.source_artifact_id,t.service_id,t.merchant_id,t.operation_fingerprint,
        t.installment_id,a.reference,a.raw_text,t.created_at,t.updated_at,t.actor_library_user_id,
        t.purchase_date
     FROM finance_transactions t LEFT JOIN finance_source_artifacts a ON a.id=t.source_artifact_id";

fn transaction_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FinanceTransaction> {
    Ok(FinanceTransaction {
        id: row.get(0)?,
        transaction_type: row.get(1)?,
        amount: row.get(2)?,
        currency: row.get(3)?,
        effective_date: row.get(4)?,
        account_id: row.get(5)?,
        destination_account_id: row.get(6)?,
        category_id: row.get(7)?,
        description: row.get(8)?,
        source: row.get(9)?,
        status: row.get(10)?,
        actor_user_id: row.get(11)?,
        source_artifact_id: row.get(12)?,
        service_id: row.get(13)?,
        merchant_id: row.get(14)?,
        operation_fingerprint: row.get(15)?,
        installment_id: row.get(16)?,
        source_reference: row.get(17)?,
        raw_source: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        actor_library_user_id: row.get(21)?,
        purchase_date: row.get(22)?,
    })
}

pub(crate) fn load_transaction(connection: &Connection, id: &str) -> Result<Option<FinanceTransaction>, String> {
    connection
        .query_row(
            &format!("{TRANSACTION_SELECT} WHERE t.id=?1 AND t.deleted_at IS NULL"),
            [id],
            transaction_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDashboard {
    pub accounts: Vec<FinanceAccount>,
    pub categories: Vec<FinanceCategory>,
    pub transactions: Vec<FinanceTransaction>,
    pub transactions_truncated: bool,
    pub income_total: String,
    pub expense_total: String,
    pub net_total: String,
    pub income_by_currency: BTreeMap<String, String>,
    pub expense_by_currency: BTreeMap<String, String>,
    pub net_by_currency: BTreeMap<String, String>,
    /// Card statements due in the month: what was paid for the cards.
    pub debt_by_currency: BTreeMap<String, String>,
    /// Services paid in the month.
    pub services_by_currency: BTreeMap<String, String>,
    /// Net salary of the previous period, the one that pays the month.
    pub salary_by_currency: BTreeMap<String, String>,
    pub debt_ratio_history: Vec<FinanceDebtRatioHistoryPoint>,
    pub savings: Vec<FinanceSavingsReserve>,
    pub savings_movements: Vec<FinanceSavingsMovement>,
    pub savings_movements_truncated: bool,
    pub merchants: Vec<FinanceMerchant>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceService {
    pub id: String,
    pub name: String,
    pub category_id: String,
    pub currency: String,
    pub expected_amount: String,
    pub due_day: Option<i64>,
    pub default_account_id: Option<String>,
    pub provider: Option<String>,
    pub modality: String,
    pub active: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceServiceOccurrence {
    pub id: String,
    pub service_id: String,
    pub period: String,
    pub expected_amount: String,
    pub paid_amount: Option<String>,
    pub effective_date: Option<String>,
    pub status: String,
    pub transaction_id: Option<String>,
    pub artifact_id: Option<String>,
    pub source_reference: Option<String>,
    pub raw_source: Option<String>,
    pub actor_library_user_id: Option<String>,
    pub source: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Paid minus expected, when the occurrence was paid.
    #[serde(default, skip_deserializing)]
    pub difference: Option<String>,
}

/// Paid minus expected amount of a paid service occurrence.
pub(crate) fn occurrence_difference(expected: &str, paid: Option<&str>) -> Option<String> {
    let paid = paid?.trim().parse::<f64>().ok().filter(|value| value.is_finite())?;
    let expected = expected.trim().parse::<f64>().ok().filter(|value| value.is_finite())?;
    Some(format!("{:.2}", paid - expected))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceServiceOccurrenceVersion {
    pub id: String,
    pub occurrence_id: String,
    pub version_number: i64,
    pub expected_amount: String,
    pub paid_amount: Option<String>,
    pub effective_date: Option<String>,
    pub status: String,
    pub transaction_id: Option<String>,
    pub artifact_id: Option<String>,
    pub source_reference: Option<String>,
    pub raw_source: Option<String>,
    pub actor_library_user_id: Option<String>,
    pub source: String,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceServiceInvoice {
    pub id: String,
    pub service_id: Option<String>,
    pub period: String,
    pub due_date: Option<String>,
    pub provider: Option<String>,
    pub amount: String,
    pub currency: String,
    pub transaction_id: Option<String>,
    pub artifact_id: Option<String>,
    pub validation_status: String,
    pub source_reference: Option<String>,
    pub raw_extraction: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceDebtRatioHistoryPoint {
    pub period: String,
    pub debt_by_currency: BTreeMap<String, String>,
    pub services_by_currency: BTreeMap<String, String>,
    pub salary_by_currency: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceMerchant {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSavingsReserve {
    pub id: String,
    pub name: String,
    pub currency: String,
    pub opening_balance: String,
    pub objective: Option<String>,
    pub active: bool,
    pub balance: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSavingsMovement {
    pub id: String,
    pub reserve_id: String,
    pub account_id: Option<String>,
    pub movement_type: String,
    pub amount: String,
    pub currency: String,
    pub effective_date: String,
    pub description: String,
    pub reason: Option<String>,
    pub source: String,
    pub status: String,
    pub actor_user_id: Option<i64>,
    #[serde(default)]
    pub actor_library_user_id: Option<String>,
    pub linked_transaction_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSavingsExchange {
    pub id: String,
    pub reserve_id: String,
    pub source_account_id: String,
    pub source_amount: String,
    pub source_currency: String,
    pub savings_amount: String,
    pub savings_currency: String,
    pub effective_date: String,
    pub description: String,
    pub actor_user_id: Option<i64>,
    pub source_reference: Option<String>,
    pub raw_source: Option<String>,
    /// `buy` (the default) moves money from the account into the reserve;
    /// `sell` takes savings out of the reserve into the account.
    #[serde(default)]
    pub direction: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinanceSavedSavingsExchange {
    pub movement: FinanceSavingsMovement,
    pub transaction: FinanceTransaction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAccountPayload {
    pub context: FinanceContext,
    pub account: FinanceAccount,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCategoryPayload {
    pub context: FinanceContext,
    pub category: FinanceCategory,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTransactionPayload {
    pub context: FinanceContext,
    pub transaction: FinanceTransaction,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinanceServicePayload {
    pub context: FinanceContext,
    pub service: FinanceService,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFinanceServiceActivePayload {
    pub context: FinanceContext,
    pub id: String,
    pub active: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinanceServiceOccurrencePayload {
    pub context: FinanceContext,
    pub occurrence: FinanceServiceOccurrence,
    pub reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinanceServiceInvoicePayload {
    pub context: FinanceContext,
    pub invoice: FinanceServiceInvoice,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSavingsReservePayload {
    pub context: FinanceContext,
    pub reserve: FinanceSavingsReserve,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSavingsMovementPayload {
    pub context: FinanceContext,
    pub movement: FinanceSavingsMovement,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSavingsExchangePayload {
    pub context: FinanceContext,
    pub exchange: FinanceSavingsExchange,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFinanceEntityPayload {
    pub context: FinanceContext,
    pub id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSavingsAccountPayload {
    pub context: FinanceContext,
    pub reserve_id: String,
    pub account_id: String,
}

const FINANCE_DEV_TABLES: &[&str] = &[
    "finance_accounts",
    "finance_categories",
    "finance_transactions",
    "finance_merchants",
    "finance_source_artifacts",
    "finance_extraction_results",
    "finance_receipts",
    "finance_purchases",
    "finance_products",
    "finance_purchase_items",
    "finance_salary_receipts",
    "finance_salary_concepts",
    "finance_savings_reserves",
    "finance_savings_movements",
    "finance_savings_accounts",
    "finance_price_observations",
    "finance_installment_plans",
    "finance_installments",
    "finance_credit_card_statements",
    "finance_credit_card_statement_items",
    "finance_services",
    "finance_service_occurrences",
    "finance_service_occurrence_versions",
    "finance_service_invoices",
    "finance_merchant_aliases",
    "finance_product_aliases",
    "finance_review_items",
    "finance_link_log",
    "finance_loans",
    "finance_loan_installments",
    "finance_budgets",
    "finance_budget_alerts",
    "finance_applied_requests",
];
const FINANCE_DEV_PAGE_SIZE: u32 = 50;
const FINANCE_DEV_MAX_PAGE_SIZE: u32 = 200;
const FINANCE_DEV_MAX_SQL_CHARS: usize = 20_000;

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

pub(crate) fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn finance_dev_page_size(value: Option<u32>) -> u32 {
    value
        .unwrap_or(FINANCE_DEV_PAGE_SIZE)
        .clamp(1, FINANCE_DEV_MAX_PAGE_SIZE)
}

fn validate_finance_dev_sql(sql: &str) -> Result<String, String> {
    let normalized = sql.trim().trim_end_matches(';').trim();
    if normalized.is_empty() || normalized.len() > FINANCE_DEV_MAX_SQL_CHARS {
        return Err("La consulta SQL debe tener entre 1 y 20000 caracteres.".to_string());
    }
    if sql.trim_end().ends_with(';') && sql.trim_end_matches(';').contains(';') {
        return Err("La consola Dev admite una única consulta SQL.".to_string());
    }
    let lower = normalized.to_ascii_lowercase();
    if !(lower.starts_with("select") || lower.starts_with("with")) {
        return Err(
            "La consola Dev solo admite consultas SELECT o WITH de solo lectura.".to_string(),
        );
    }
    for keyword in [
        "insert", "update", "delete", "replace", "drop", "alter", "create", "attach", "detach",
        "vacuum", "pragma",
    ] {
        if lower
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|token| token == keyword)
        {
            return Err(
                "La consola Dev no admite sentencias que modifiquen la base de datos.".to_string(),
            );
        }
    }
    Ok(normalized.to_string())
}

fn finance_dev_value(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(value) => Some(value.to_string()),
        ValueRef::Real(value) => Some(value.to_string()),
        ValueRef::Text(value) => Some(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Some(format!("<blob: {} bytes>", value.len())),
    }
}

fn finance_dev_query(
    connection: &Connection,
    sql: &str,
    page: u32,
    page_size: u32,
) -> Result<FinanceDevQueryResult, String> {
    let total_rows: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM ({sql}) AS finance_dev_count"),
            [],
            |row| row.get(0),
        )
        .map_err(|_| "No se pudo contar el resultado de la consulta SQL.".to_string())?;
    let offset = i64::from(page) * i64::from(page_size);
    let mut statement = connection
        .prepare(&format!(
            "SELECT * FROM ({sql}) AS finance_dev_result LIMIT ?1 OFFSET ?2"
        ))
        .map_err(|_| "La consulta SQL no es válida.".to_string())?;
    let columns = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let rows = statement
        .query_map(params![page_size, offset], |row| {
            (0..row.as_ref().column_count())
                .map(|index| row.get_ref(index).map(finance_dev_value))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| "No se pudo ejecutar la consulta SQL.".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "No se pudo leer el resultado de la consulta SQL.".to_string())?;
    Ok(FinanceDevQueryResult {
        columns,
        rows,
        total_rows,
        page,
        page_size,
    })
}

pub fn finance_dev_list_tables() -> Vec<FinanceDevTable> {
    FINANCE_DEV_TABLES
        .iter()
        .map(|name| FinanceDevTable { name })
        .collect()
}

pub fn finance_dev_query_table(
    app: crate::host::AppHandle,
    payload: FinanceDevTableQueryPayload,
) -> FinanceCommandResult<FinanceDevQueryResult> {
    if !FINANCE_DEV_TABLES.contains(&payload.table_name.as_str()) {
        return Err("La entidad financiera solicitada no existe.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    finance_dev_query(
        &connection,
        &format!("SELECT * FROM {}", payload.table_name),
        payload.page,
        finance_dev_page_size(payload.page_size),
    )
    .map_err(Into::into)
}

pub fn finance_dev_query_sql(
    app: crate::host::AppHandle,
    payload: FinanceDevSqlQueryPayload,
) -> FinanceCommandResult<FinanceDevQueryResult> {
    let sql = validate_finance_dev_sql(&payload.sql)?;
    let connection = validate_context(&payload.context, &app)?;
    finance_dev_query(
        &connection,
        &sql,
        payload.page,
        finance_dev_page_size(payload.page_size),
    )
    .map_err(Into::into)
}

pub(crate) fn seed_finance_demo_data(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute_batch(
            "INSERT OR IGNORE INTO finance_accounts (id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES
                ('dev-account-cash','Efectivo demo','cash','ARS','12500.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-account-bank','Cuenta bancaria demo','bank','ARS','85000.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-account-card','Tarjeta demo','credit_card','ARS','0.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-account-savings','Caja de ahorro demo','savings_reserve','ARS','0.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-account-savings-usd','Ahorro en dólares demo','savings_reserve','USD','0.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-account-usd','Cuenta USD inactiva','bank','USD','150.00',0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_categories (id,name,kind,parent_id,active,description,created_at,updated_at) VALUES
                ('dev-category-food','Alimentos','expense',NULL,1,'Compras de comida y supermercado.',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-category-coffee','Cafetería','expense','dev-category-food',1,'Consumos pequeños fuera de casa.',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-category-transport','Transporte','expense',NULL,1,'Traslados cotidianos.',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-category-salary','Sueldo','income',NULL,1,'Ingresos por empleo.',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-category-old','Categoría inactiva','expense',NULL,0,'Ejemplo de categoría desactivada.',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_merchants (id,name,normalized_name,created_at,updated_at) VALUES
                ('dev-merchant-market','Mercado Central','mercado central',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-merchant-coffee','Café de la Plaza','cafe de la plaza',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_source_artifacts (id,source_type,reference,raw_text,content_hash,created_at) VALUES
                ('dev-art-ticket-jul','ticket','ticket-demo-julio.pdf','Ticket demo de supermercado de julio.','demo-ticket-jul',CURRENT_TIMESTAMP),
                ('dev-art-ticket-aug','ticket','ticket-demo-agosto.pdf','Ticket demo de supermercado de agosto.','demo-ticket-aug',CURRENT_TIMESTAMP),
                ('dev-art-salary-jul','salary','recibo-demo-julio.pdf','Recibo de sueldo demo julio.','demo-salary-jul',CURRENT_TIMESTAMP),
                ('dev-art-salary-aug','salary','recibo-demo-agosto.pdf','Recibo de sueldo demo agosto.','demo-salary-aug',CURRENT_TIMESTAMP),
                ('dev-art-card-jul','credit_card_statement','resumen-demo-julio.pdf','Resumen de tarjeta demo julio.','demo-card-jul',CURRENT_TIMESTAMP),
                ('dev-art-card-aug','credit_card_statement','resumen-demo-agosto.pdf','Resumen de tarjeta demo agosto.','demo-card-aug',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_transactions (id,transaction_type,amount,currency,effective_date,purchase_date,account_id,destination_account_id,category_id,description,source,status,source_artifact_id,merchant_id,operation_fingerprint,installment_id,created_at,updated_at) VALUES
                ('dev-tx-salary-jul','income','120000.00','ARS','2026-07-31',NULL,'dev-account-bank',NULL,'dev-category-salary','Sueldo julio 2026','salary','confirmed','dev-art-salary-jul',NULL,'demo-salary-jul',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-ticket-jul','expense','18500.00','ARS','2026-07-18',NULL,'dev-account-bank',NULL,'dev-category-food','Compra supermercado julio','ticket','confirmed','dev-art-ticket-jul','dev-merchant-market','demo-ticket-jul',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-card-jul','expense','12000.00','ARS','2026-08-05','2026-07-20','dev-account-card',NULL,'dev-category-transport','Viaje demo julio','credit_card_statement','confirmed',NULL,NULL,'demo-card-jul',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-salary-aug','income','128000.00','ARS','2026-08-28',NULL,'dev-account-bank',NULL,'dev-category-salary','Sueldo agosto 2026','salary','confirmed','dev-art-salary-aug',NULL,'demo-salary-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-ticket-aug','expense','24100.00','ARS','2026-08-12',NULL,'dev-account-bank',NULL,'dev-category-food','Compra supermercado agosto','ticket','corrected','dev-art-ticket-aug','dev-merchant-market','demo-ticket-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-transfer-aug','transfer','30000.00','ARS','2026-08-15',NULL,'dev-account-bank','dev-account-savings',NULL,'Aporte a ahorro','manual','confirmed',NULL,NULL,'demo-transfer-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-exchange-aug','exchange','145000.00','ARS','2026-08-02',NULL,'dev-account-bank','dev-account-savings-usd',NULL,'Compra de USD para ahorro','savings_exchange','confirmed',NULL,NULL,'demo-exchange-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-installment-aug','expense','15000.00','ARS','2026-09-05','2026-08-10','dev-account-card',NULL,'dev-category-food','Cuota 1 de compra demo','credit_card_statement','confirmed',NULL,'dev-merchant-market','demo-installment-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-card-unpaid','expense','8000.00','ARS','2026-09-12','2026-09-12','dev-account-card',NULL,'dev-category-coffee','Compra en Café de la Plaza','telegram','card_unpaid',NULL,'dev-merchant-coffee','demo-card-unpaid',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-tx-coffee-aug','expense','3500.00','ARS','2026-08-22',NULL,'dev-account-cash',NULL,'dev-category-coffee','Café demo','manual','discarded',NULL,'dev-merchant-coffee','demo-coffee-aug',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_extraction_results (id,source_artifact_id,extractor,raw_result,confidence,status,created_at) VALUES
                ('dev-extraction-ticket-jul','dev-art-ticket-jul','demo','{\"merchant\":\"Mercado Central\",\"total\":18500}',0.98,'confirmed',CURRENT_TIMESTAMP),
                ('dev-extraction-salary-aug','dev-art-salary-aug','demo','{\"employer\":\"Empresa Demo SA\",\"net\":128000}',0.96,'confirmed',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_receipts (id,source_artifact_id,receipt_type,validation_status,created_at,updated_at) VALUES
                ('dev-receipt-ticket-jul','dev-art-ticket-jul','ticket','confirmed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-receipt-salary-aug','dev-art-salary-aug','salary','corrected',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_purchases (id,transaction_id,merchant_id,observed_at,currency,total_amount,source_artifact_id,subtotal_amount,discount_amount,tax_amount,validation_status,created_at,updated_at) VALUES
                ('dev-purchase-jul','dev-tx-ticket-jul','dev-merchant-market','2026-07-18','ARS','18500.00','dev-art-ticket-jul','19000.00','500.00','0.00','confirmed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-purchase-aug','dev-tx-ticket-aug','dev-merchant-market','2026-08-12','ARS','24100.00','dev-art-ticket-aug','24100.00','0.00','0.00','corrected',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_products (id,name,normalized_name,created_at,updated_at) VALUES
                ('dev-product-milk','Leche entera 1 L','leche entera 1 l',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-product-bread','Pan lactal','pan lactal',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-product-milk-short','LECHE ENT 1LT','leche ent 1l',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_purchase_items (id,purchase_id,product_id,original_description,normalized_description,quantity,unit_price,discount_amount,line_total,currency,category_id,created_at) VALUES
                ('dev-item-jul-milk','dev-purchase-jul','dev-product-milk','Leche entera 1 L','leche entera 1 l','2','4500.00','0.00','9000.00','ARS','dev-category-food',CURRENT_TIMESTAMP),
                ('dev-item-jul-bread','dev-purchase-jul','dev-product-bread','Pan lactal','pan lactal','1','10000.00','500.00','9500.00','ARS','dev-category-food',CURRENT_TIMESTAMP),
                ('dev-item-aug-milk','dev-purchase-aug','dev-product-milk','Leche entera 1 L','leche entera 1 l','2','5200.00','0.00','10400.00','ARS','dev-category-food',CURRENT_TIMESTAMP),
                ('dev-item-aug-bread','dev-purchase-aug','dev-product-bread','Pan lactal','pan lactal','1','13700.00','0.00','13700.00','ARS','dev-category-food',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_price_observations (id,purchase_item_id,product_id,merchant_id,observed_at,currency,quantity,unit_price,discount_amount,final_amount,status,created_at) VALUES
                ('dev-price-jul-milk','dev-item-jul-milk','dev-product-milk','dev-merchant-market','2026-07-18','ARS','2','4500.00','0.00','9000.00','confirmed',CURRENT_TIMESTAMP),
                ('dev-price-aug-milk','dev-item-aug-milk','dev-product-milk','dev-merchant-market','2026-08-12','ARS','2','5200.00','0.00','10400.00','corrected',CURRENT_TIMESTAMP),
                ('dev-price-aug-bread','dev-item-aug-bread','dev-product-bread','dev-merchant-market','2026-08-12','ARS','1','13700.00','0.00','13700.00','pending',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_salary_receipts (id,period,payment_date,employer,gross_amount,deductions_total,net_amount,currency,account_id,transaction_id,source_artifact_id,validation_status,created_at,updated_at) VALUES
                ('dev-salary-jul','2026-07','2026-07-31','Empresa Demo SA','150000.00','30000.00','120000.00','ARS','dev-account-bank','dev-tx-salary-jul','dev-art-salary-jul','confirmed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-salary-aug','2026-08','2026-08-28','Empresa Demo SA','160000.00','32000.00','128000.00','ARS','dev-account-bank','dev-tx-salary-aug','dev-art-salary-aug','corrected',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_salary_concepts (id,salary_receipt_id,name,concept_type,amount,currency,created_at) VALUES
                ('dev-salary-concept-jul-gross','dev-salary-jul','Sueldo básico','earning','150000.00','ARS',CURRENT_TIMESTAMP),
                ('dev-salary-concept-jul-deduction','dev-salary-jul','Aportes jubilatorios','deduction','30000.00','ARS',CURRENT_TIMESTAMP),
                ('dev-salary-concept-aug-gross','dev-salary-aug','Sueldo básico','earning','160000.00','ARS',CURRENT_TIMESTAMP),
                ('dev-salary-concept-aug-deduction','dev-salary-aug','Aportes jubilatorios','deduction','32000.00','ARS',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_savings_reserves (id,name,currency,opening_balance,objective,active,ledger_account_id,created_at,updated_at) VALUES
                ('dev-reserve-emergency','Fondo de emergencia','ARS','50000.00','Cubrir tres meses de gastos.',1,'dev-account-savings',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-reserve-usd','Ahorros','USD','900.00','Ahorro en dólares.',1,'dev-account-savings-usd',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_savings_accounts (reserve_id,account_id,created_at) VALUES
                ('dev-reserve-emergency','dev-account-bank',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_savings_movements (id,reserve_id,account_id,movement_type,amount,currency,effective_date,description,reason,source,status,linked_transaction_id,created_at,updated_at) VALUES
                ('dev-savings-jul','dev-reserve-emergency','dev-account-bank','contribution','10000.00','ARS','2026-07-25','Aporte mensual',NULL,'manual','confirmed',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-savings-aug','dev-reserve-emergency','dev-account-bank','contribution','30000.00','ARS','2026-08-15','Aporte desde cuenta bancaria',NULL,'manual','confirmed','dev-tx-transfer-aug',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-savings-loss','dev-reserve-emergency',NULL,'loss','500.00','ARS','2026-08-20','Ajuste demo','Diferencia de caja','manual','pending',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('savings-exchange:dev-tx-exchange-aug','dev-reserve-usd',NULL,'contribution','100.00','USD','2026-08-02','Compra de USD para ahorro',NULL,'savings_exchange','confirmed','dev-tx-exchange-aug',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_installment_plans (id,account_id,merchant_id,description,purchase_date,currency,total_amount,installment_count,created_at,updated_at) VALUES
                ('dev-plan-market','dev-account-card','dev-merchant-market','Compra demo en 3 cuotas','2026-08-10','ARS','45000.00',3,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_installments (id,plan_id,installment_number,due_date,amount,status,transaction_id,created_at,updated_at) VALUES
                ('dev-installment-1','dev-plan-market',1,'2026-09-05','15000.00','confirmed','dev-tx-installment-aug',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-installment-2','dev-plan-market',2,'2026-10-05','15000.00','pending',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-installment-3','dev-plan-market',3,'2026-11-05','15000.00','pending',NULL,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_credit_card_statements (id,account_id,issuer,card_last_four,period,closing_date,due_date,currency,previous_balance,payments_amount,credits_amount,purchases_amount,fees_amount,interest_amount,taxes_amount,total_due,minimum_payment,source_artifact_id,validation_status,created_at,updated_at) VALUES
                ('dev-statement-jul','dev-account-card','Banco Demo','1234','2026-07','2026-07-25','2026-08-05','ARS','0.00','0.00','0.00','12000.00','500.00','0.00','0.00','12500.00','1250.00','dev-art-card-jul','confirmed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-statement-aug','dev-account-card','Banco Demo','1234','2026-08','2026-08-25','2026-09-05','ARS','12500.00','12500.00','0.00','15000.00','0.00','250.00','0.00','15250.00','1525.00','dev-art-card-aug','corrected',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_credit_card_statement_items (id,statement_id,transaction_id,purchase_date,description,amount,currency,item_type,installment_number,installment_count,created_at) VALUES
                ('dev-statement-item-jul-purchase','dev-statement-jul','dev-tx-card-jul','2026-07-20','Viaje demo julio','12000.00','ARS','purchase',NULL,NULL,CURRENT_TIMESTAMP),
                ('dev-statement-item-jul-fee','dev-statement-jul',NULL,'2026-07-25','Mantenimiento','500.00','ARS','fee',NULL,NULL,CURRENT_TIMESTAMP),
                ('dev-statement-item-aug-installment','dev-statement-aug','dev-tx-installment-aug','2026-08-10','Cuota 1 compra demo','15000.00','ARS','purchase',1,3,CURRENT_TIMESTAMP),
                ('dev-statement-item-aug-interest','dev-statement-aug',NULL,'2026-08-25','Interés demo','250.00','ARS','interest',NULL,NULL,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_services (id,name,normalized_name,category_id,currency,expected_amount,due_day,default_account_id,provider,normalized_provider,modality,active,created_at,updated_at) VALUES
                ('dev-service-internet','Internet demo','internet demo','dev-category-food','ARS','18000.00',10,'dev-account-bank','Fibra Demo','fibra demo','fixed',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-service-power','Luz demo','luz demo','dev-category-food','ARS','9500.00',15,'dev-account-bank','Energía Demo','energia demo','variable',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_service_occurrences (id,service_id,period,expected_amount,paid_amount,effective_date,status,source,created_at,updated_at) VALUES
                ('dev-occurrence-internet-jul','dev-service-internet','2026-07','18000.00','18000.00','2026-07-10','accepted','dev-seed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-occurrence-internet-aug','dev-service-internet','2026-08','18000.00',NULL,NULL,'pending','dev-seed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-occurrence-power-aug','dev-service-power','2026-08','9500.00','10240.00','2026-08-15','accepted','dev-seed',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_service_occurrence_versions (id,occurrence_id,version_number,expected_amount,paid_amount,effective_date,status,source,reason,created_at) VALUES
                ('dev-occurrence-internet-jul-v1','dev-occurrence-internet-jul',1,'18000.00',NULL,NULL,'pending','dev-seed','Alta del período',CURRENT_TIMESTAMP),
                ('dev-occurrence-internet-jul-v2','dev-occurrence-internet-jul',2,'18000.00','18000.00','2026-07-10','accepted','dev-seed','Pago registrado',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_service_invoices (id,service_id,period,due_date,provider,amount,currency,validation_status,created_at,updated_at) VALUES
                ('dev-invoice-power-aug','dev-service-power','2026-08','2026-08-15','Energía Demo','10240.00','ARS','valid',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_merchant_aliases (normalized_alias,merchant_id) VALUES
                ('mercado central','dev-merchant-market'),
                ('cafe de la plaza','dev-merchant-coffee');
             INSERT OR IGNORE INTO finance_product_aliases (normalized_alias,product_id) VALUES
                ('leche entera 1l','dev-product-milk'),
                ('pan lactal','dev-product-bread');
             INSERT OR IGNORE INTO finance_review_items (id,kind,subject_key,status,question,options_json,subject_json,created_at) VALUES
                ('dev-review-product','similar-product','dev-product-milk-short|dev-product-milk','pending','¿«LECHE ENT 1LT» es el mismo producto que «Leche entera 1 L»?','[{\"id\":\"merge\",\"label\":\"Sí, es «Leche entera 1 L»\"},{\"id\":\"keep\",\"label\":\"No, son distintos\"}]','{\"newId\":\"dev-product-milk-short\",\"existingId\":\"dev-product-milk\"}',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_link_log (id,kind,subject_id,target_id,summary,created_at) VALUES
                ('dev-link-installment','installment-card-line','dev-installment-1','dev-statement-item-aug-installment','La línea «Cuota 1 compra demo» pagó la cuota 1/3.',CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_loans (id,name,lender,currency,principal,installment_count,installment_amount,first_due_date,account_id,notes,active,created_at,updated_at) VALUES
                ('dev-loan-car','Préstamo demo del auto','Banco Demo','ARS','100000.00',2,'55000.00','2026-10-10','dev-account-bank',NULL,1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_loan_installments (id,loan_id,number,due_date,expected_amount,created_at,updated_at) VALUES
                ('dev-loan-car:1','dev-loan-car',1,'2026-10-10','55000.00',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP),
                ('dev-loan-car:2','dev-loan-car',2,'2026-11-10','55000.00',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
             INSERT OR IGNORE INTO finance_budgets (id,category_id,currency,amount,active,created_at,updated_at) VALUES
                ('dev-budget-food','dev-category-food','ARS','30000.00',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);",
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn finance_dev_seed_demo_data(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<()> {
    let mut connection = validate_context(&context, &app)?;
    seed_finance_demo_data(&mut connection)?;
    sync_context(&context, &app)?;
    Ok(())
}
pub(crate) fn validate_context(
    context: &FinanceContext,
    app: &crate::host::AppHandle,
) -> Result<Connection, String> {
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
    // `manual`: a form of the Finanzas screen; `android`: the Notia Finanzas app.
    if !matches!(context.source.as_str(), "app" | "public-url" | "telegram" | "manual" | "android") {
        return Err("El origen de la operación financiera no es válido.".to_string());
    }
    let actor_id = context.actor_library_user_id.trim();
    if actor_id.is_empty() {
        return Err(
            "El usuario de la biblioteca es obligatorio para acceder a Finanzas.".to_string(),
        );
    }
    let role_id: Option<String> = connection
        .query_row(
            "SELECT role_id FROM library_users WHERE id=?1",
            [actor_id],
            |row| row.get(0),
        )
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
            return Err(
                "El usuario no tiene el contexto #Confidencial requerido para Finanzas."
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// Event the Finanzas screen listens to, to read its figures again.
pub const FINANCE_DATA_CHANGED_EVENT: &str = "notia:finance-data-changed";

/// Every finance write ends here: it saves the Android copy back to the
/// library and tells the open screen that the records changed.
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
    #[cfg(not(target_os = "android"))]
    let _ = context;
    use crate::host::Emitter;
    if let Err(error) = app.emit(FINANCE_DATA_CHANGED_EVENT, ()) {
        log::warn!("[notia:finance] evento de cambio no emitido: {error}");
    }
    // A change may take a category past its budget: the Owner hears it.
    crate::finance_budgets::check_alerts_later(app, context);
    Ok(())
}
fn valid_amount(value: &str) -> bool {
    parse_cents(value).is_ok()
}

pub(crate) fn parse_cents(value: &str) -> Result<i128, ()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 32 {
        return Err(());
    }
    let mut parts = trimmed.split('.');
    let whole = parts.next().ok_or(())?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err(());
    }
    let cents = format!("{fraction:0<2}").parse::<i128>().map_err(|_| ())?;
    let result = whole
        .parse::<i128>()
        .map_err(|_| ())?
        .checked_mul(100)
        .and_then(|value| value.checked_add(cents))
        .ok_or(())?;
    Ok(result)
}

pub(crate) fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes[..4]
            .iter()
            .chain(&bytes[5..7])
            .chain(&bytes[8..])
            .all(u8::is_ascii_digit)
    {
        return false;
    }
    let year = value[0..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..10].parse::<u32>().unwrap_or(0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    };
    days_in_month > 0 && (1..=days_in_month).contains(&day)
}

fn valid_bounded_text(value: &str, max: usize, required: bool) -> bool {
    let trimmed = value.trim();
    (!required || !trimmed.is_empty()) && trimmed.chars().count() <= max
}

pub(crate) fn format_cents(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let absolute = value.abs();
    format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
}

fn add_currency_total(totals: &mut BTreeMap<String, i128>, currency: &str, amount: &str) {
    if let Ok(cents) = parse_cents(amount) {
        *totals.entry(currency.to_string()).or_default() += cents;
    }
}

fn apply_savings_movement(balance: &mut i128, movement_type: &str, amount: &str, status: &str) {
    if !matches!(status, "confirmed" | "corrected") {
        return;
    }
    let amount = parse_cents(amount).unwrap_or_default();
    if matches!(movement_type, "contribution" | "return" | "adjustment") {
        *balance += amount;
    } else if matches!(movement_type, "withdrawal" | "loss") {
        *balance -= amount;
    }
}

pub(crate) fn valid_service_period(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(|byte| byte.is_ascii_digit())
        && bytes[5..].iter().all(|byte| byte.is_ascii_digit())
        && (1..=12).contains(&value[5..].parse::<u8>().unwrap_or(0))
}

fn validate_service(service: &FinanceService, connection: &Connection) -> Result<(), String> {
    if service.id.trim().is_empty()
        || service.name.trim().is_empty()
        || service.name.chars().count() > 160
        || !valid_amount(&service.expected_amount)
        || !matches!(service.currency.as_str(), "ARS" | "USD")
        || !matches!(service.modality.as_str(), "fixed" | "variable")
        || service
            .due_day
            .map(|day| !(1..=31).contains(&day))
            .unwrap_or(false)
    {
        return Err("El servicio requiere nombre, importe, moneda y modalidad válidos.".into());
    }
    if service.provider.as_deref().map(str::len).unwrap_or(0) > 160 {
        return Err("El proveedor del servicio es demasiado largo.".into());
    }
    let category_kind: String = connection
        .query_row(
            "SELECT kind FROM finance_categories WHERE id=?1 AND active=1",
            [&service.category_id],
            |row| row.get(0),
        )
        .map_err(|_| "La categoría del servicio no existe o está inactiva.".to_string())?;
    if category_kind != "expense" {
        return Err("Un servicio solo puede usar una categoría de gasto.".into());
    }
    if let Some(account_id) = service
        .default_account_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let account_currency: String = connection
            .query_row(
                "SELECT currency FROM finance_accounts WHERE id=?1 AND active=1",
                [account_id],
                |row| row.get(0),
            )
            .map_err(|_| "La cuenta predeterminada no existe o está inactiva.".to_string())?;
        if account_currency != service.currency {
            return Err("La cuenta predeterminada debe usar la moneda del servicio.".into());
        }
    }
    Ok(())
}

fn service_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FinanceService> {
    Ok(FinanceService {
        id: row.get(0)?,
        name: row.get(1)?,
        category_id: row.get(2)?,
        currency: row.get(3)?,
        expected_amount: row.get(4)?,
        due_day: row.get(5)?,
        default_account_id: row.get(6)?,
        provider: row.get(7)?,
        modality: row.get(8)?,
        active: row.get::<_, i32>(9)? != 0,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn occurrence_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FinanceServiceOccurrence> {
    Ok(FinanceServiceOccurrence {
        id: row.get(0)?,
        service_id: row.get(1)?,
        period: row.get(2)?,
        expected_amount: row.get(3)?,
        paid_amount: row.get(4)?,
        effective_date: row.get(5)?,
        status: row.get(6)?,
        transaction_id: row.get(7)?,
        artifact_id: row.get(8)?,
        source_reference: row.get(9)?,
        raw_source: row.get(10)?,
        actor_library_user_id: row.get(11)?,
        source: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        difference: None,
    })
    .map(|mut occurrence: FinanceServiceOccurrence| {
        occurrence.difference = occurrence_difference(&occurrence.expected_amount, occurrence.paid_amount.as_deref());
        occurrence
    })
}

fn transaction_query_occurrence(
    connection: &Connection,
    service_id: &str,
    period: &str,
) -> Result<FinanceServiceOccurrence, String> {
    connection
        .query_row(
            "SELECT id,service_id,period,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,created_at,updated_at FROM finance_service_occurrences WHERE service_id=?1 AND period=?2",
            params![service_id, period],
            occurrence_from_row,
        )
        .map_err(|error| error.to_string())
}

pub fn finance_list_services(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceService>> {
    let connection = validate_context(&context, &app)?;
    let result = connection
        .prepare("SELECT id,name,category_id,currency,expected_amount,due_day,default_account_id,provider,modality,active,created_at,updated_at FROM finance_services ORDER BY active DESC, name")
        .map_err(|error| error.to_string())?
        .query_map([], service_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string().into());
    result
}

pub fn finance_save_service(
    app: crate::host::AppHandle,
    payload: SaveFinanceServicePayload,
) -> FinanceCommandResult<FinanceService> {
    let connection = validate_context(&payload.context, &app)?;
    validate_service(&payload.service, &connection)?;
    let timestamp = now();
    let provider = payload
        .service
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let normalized_name = normalize_service_text(&payload.service.name);
    let normalized_provider = provider.map(normalize_service_text);
    connection.execute(
        "INSERT INTO finance_services(id,name,normalized_name,category_id,currency,expected_amount,due_day,default_account_id,provider,normalized_provider,modality,active,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name,normalized_name=excluded.normalized_name,category_id=excluded.category_id,currency=excluded.currency,expected_amount=excluded.expected_amount,due_day=excluded.due_day,default_account_id=excluded.default_account_id,provider=excluded.provider,normalized_provider=excluded.normalized_provider,modality=excluded.modality,active=excluded.active,updated_at=excluded.updated_at",
        params![payload.service.id, payload.service.name.trim(), normalized_name, payload.service.category_id, payload.service.currency, payload.service.expected_amount, payload.service.due_day, payload.service.default_account_id, provider, normalized_provider, payload.service.modality, i32::from(payload.service.active), timestamp],
    ).map_err(|error| if error.to_string().contains("UNIQUE") { FinanceCommandError::from("Ya existe un servicio con ese nombre y proveedor.") } else { error.to_string().into() })?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    let connection = validate_context(&payload.context, &app)?;
    let persisted = connection.query_row("SELECT id,name,category_id,currency,expected_amount,due_day,default_account_id,provider,modality,active,created_at,updated_at FROM finance_services WHERE id=?1", [&payload.service.id], service_from_row).map_err(|error| error.to_string())?;
    Ok(persisted)
}

pub fn finance_set_service_active(
    app: crate::host::AppHandle,
    payload: SetFinanceServiceActivePayload,
) -> FinanceCommandResult<()> {
    let connection = validate_context(&payload.context, &app)?;
    let changed = connection
        .execute(
            "UPDATE finance_services SET active=?1,updated_at=?2 WHERE id=?3",
            params![i32::from(payload.active), now(), payload.id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("El servicio no existe.".into());
    }
    drop(connection);
    sync_context(&payload.context, &app).map_err(FinanceCommandError::from)
}

pub fn finance_list_service_occurrences(
    app: crate::host::AppHandle,
    context: FinanceContext,
    period: String,
) -> FinanceCommandResult<Vec<FinanceServiceOccurrence>> {
    if !valid_service_period(&period) {
        return Err("El período debe tener formato YYYY-MM.".into());
    }
    let connection = validate_context(&context, &app)?;
    let result = connection.prepare("SELECT id,service_id,period,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,created_at,updated_at FROM finance_service_occurrences WHERE period=?1 ORDER BY service_id").map_err(|error| error.to_string())?
        .query_map([period], occurrence_from_row).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string().into());
    result
}

pub fn finance_list_all_service_occurrences(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceServiceOccurrence>> {
    let connection = validate_context(&context, &app)?;
    let result = connection
        .prepare("SELECT id,service_id,period,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,created_at,updated_at FROM finance_service_occurrences ORDER BY period DESC,service_id")
        .map_err(|error| error.to_string())?
        .query_map([], occurrence_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string().into());
    result
}

/// Saves a service's occurrence for a period, or replaces the one it had;
/// the replaced row keeps its `created_at`. `?14` is the timestamp.
const UPSERT_SERVICE_OCCURRENCE_SQL: &str = "INSERT INTO finance_service_occurrences(id,service_id,period,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,COALESCE((SELECT created_at FROM finance_service_occurrences WHERE service_id=?2 AND period=?3),?14),?14) ON CONFLICT(service_id,period) DO UPDATE SET expected_amount=excluded.expected_amount,paid_amount=excluded.paid_amount,effective_date=excluded.effective_date,status=excluded.status,transaction_id=excluded.transaction_id,artifact_id=excluded.artifact_id,source_reference=excluded.source_reference,raw_source=excluded.raw_source,actor_library_user_id=excluded.actor_library_user_id,source=excluded.source,updated_at=excluded.updated_at";

pub fn finance_save_service_occurrence(
    app: crate::host::AppHandle,
    payload: SaveFinanceServiceOccurrencePayload,
) -> FinanceCommandResult<FinanceServiceOccurrence> {
    let occurrence = &payload.occurrence;
    if occurrence.id.trim().is_empty()
        || !valid_service_period(&occurrence.period)
        || !valid_amount(&occurrence.expected_amount)
        || occurrence
            .paid_amount
            .as_deref()
            .map(|value| !valid_amount(value))
            .unwrap_or(false)
        || occurrence
            .effective_date
            .as_deref()
            .map(|value| !valid_iso_date(value))
            .unwrap_or(false)
        || !matches!(
            occurrence.status.as_str(),
            "pending" | "current" | "accepted" | "rejected" | "discarded" | "failed" | "outdated"
        )
        || occurrence.source.trim().is_empty()
    {
        return Err("La ocurrencia requiere período, importe y estado válidos.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let service_currency: String = connection
        .query_row(
            "SELECT currency FROM finance_services WHERE id=?1",
            [&occurrence.service_id],
            |row| row.get(0),
        )
        .map_err(|_| "El servicio no existe.".to_string())?;
    if let Some(transaction_id) = occurrence.transaction_id.as_deref() {
        let (transaction_currency, transaction_type): (String, String) = connection
            .query_row("SELECT currency,transaction_type FROM finance_transactions WHERE id=?1 AND deleted_at IS NULL", [transaction_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| "La transacción asociada no existe.".to_string())?;
        if transaction_currency != service_currency {
            return Err("La transacción asociada debe usar la moneda del servicio.".into());
        }
        if transaction_type != "expense" {
            return Err("La ocurrencia solo puede vincularse a un gasto.".into());
        }
        let transaction_amount: String = connection
            .query_row(
                "SELECT amount FROM finance_transactions WHERE id=?1 AND deleted_at IS NULL",
                [transaction_id],
                |row| row.get(0),
            )
            .map_err(|_| "La transacción asociada no existe.".to_string())?;
        if occurrence
            .paid_amount
            .as_deref()
            .and_then(|value| parse_cents(value).ok())
            != parse_cents(&transaction_amount).ok()
        {
            return Err("El importe pagado debe coincidir con el gasto asociado.".into());
        }
        let already_linked: Option<String> = connection
            .query_row(
                "SELECT service_id FROM finance_service_occurrences WHERE transaction_id=?1 AND NOT (service_id=?2 AND period=?3)",
                params![transaction_id, occurrence.service_id, occurrence.period],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if already_linked.is_some() {
            return Err("El gasto ya está vinculado a otra ocurrencia de servicio.".into());
        }
    }
    if let Some(artifact_id) = occurrence.artifact_id.as_deref() {
        let artifact_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM finance_source_artifacts WHERE id=?1)",
                [artifact_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !artifact_exists {
            return Err("El artefacto asociado no existe.".into());
        }
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let old: Option<FinanceServiceOccurrence> = transaction.query_row("SELECT id,service_id,period,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,created_at,updated_at FROM finance_service_occurrences WHERE service_id=?1 AND period=?2", params![occurrence.service_id, occurrence.period], occurrence_from_row).optional().map_err(|error| error.to_string())?;
    if let Some(persisted) = old.as_ref().filter(|previous| {
        let same_expected_amount = parse_cents(&previous.expected_amount).ok()
            == parse_cents(&occurrence.expected_amount).ok();
        let same_paid_amount = match (&previous.paid_amount, &occurrence.paid_amount) {
            (Some(previous), Some(current)) => parse_cents(previous) == parse_cents(current),
            (None, None) => true,
            _ => false,
        };
        same_expected_amount
            && same_paid_amount
            && previous.effective_date == occurrence.effective_date
            && previous.status == occurrence.status
            && previous.transaction_id == occurrence.transaction_id
            && previous.artifact_id == occurrence.artifact_id
            && previous.source_reference == occurrence.source_reference
            && previous.raw_source == occurrence.raw_source
    }) {
        let persisted = persisted.clone();
        if let Some(transaction_id) = occurrence.transaction_id.as_deref() {
            transaction
                .execute(
                    "UPDATE finance_transactions SET service_id=?1 WHERE id=?2 AND deleted_at IS NULL",
                    params![occurrence.service_id, transaction_id],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE finance_purchases SET service_id=?1,updated_at=?2 WHERE transaction_id=?3",
                    params![occurrence.service_id, now(), transaction_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        drop(connection);
        sync_context(&payload.context, &app)?;
        return Ok(persisted);
    }
    if let Some(transaction_id) = occurrence.transaction_id.as_deref() {
        if let Some(previous) = old
            .as_ref()
            .and_then(|value| value.transaction_id.as_deref())
        {
            if previous != transaction_id {
                transaction.execute("UPDATE finance_transactions SET service_id=NULL WHERE id=?1 AND service_id=?2", params![previous, occurrence.service_id]).map_err(|error| error.to_string())?;
                transaction.execute("UPDATE finance_purchases SET service_id=NULL,updated_at=?1 WHERE transaction_id=?2 AND service_id=?3", params![now(), previous, occurrence.service_id]).map_err(|error| error.to_string())?;
            }
        }
        let changed = transaction
            .execute(
                "UPDATE finance_transactions SET service_id=?1 WHERE id=?2 AND deleted_at IS NULL",
                params![occurrence.service_id, transaction_id],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("La transacción asociada no existe.".into());
        }
        transaction
            .execute(
                "UPDATE finance_purchases SET service_id=?1,updated_at=?2 WHERE transaction_id=?3",
                params![occurrence.service_id, now(), transaction_id],
            )
            .map_err(|error| error.to_string())?;
    } else if let Some(previous) = old
        .as_ref()
        .and_then(|value| value.transaction_id.as_deref())
    {
        transaction
            .execute(
                "UPDATE finance_transactions SET service_id=NULL WHERE id=?1 AND service_id=?2",
                params![previous, occurrence.service_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.execute("UPDATE finance_purchases SET service_id=NULL,updated_at=?1 WHERE transaction_id=?2 AND service_id=?3", params![now(), previous, occurrence.service_id]).map_err(|error| error.to_string())?;
    }
    let occurrence_id = old
        .as_ref()
        .map(|previous| previous.id.clone())
        .unwrap_or_else(|| occurrence.id.clone());
    let timestamp = now();
    if let Some(previous) = old {
        let version: i64 = transaction.query_row("SELECT COALESCE(MAX(version_number),0)+1 FROM finance_service_occurrence_versions WHERE occurrence_id=?1", [&previous.id], |row| row.get(0)).map_err(|error| error.to_string())?;
        transaction.execute("INSERT INTO finance_service_occurrence_versions(id,occurrence_id,version_number,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)", params![new_id(), previous.id, version, previous.expected_amount, previous.paid_amount, previous.effective_date, previous.status, previous.transaction_id, previous.artifact_id, previous.source_reference, previous.raw_source, previous.actor_library_user_id, previous.source, payload.reason, timestamp]).map_err(|error| error.to_string())?;
    }
    transaction.execute(UPSERT_SERVICE_OCCURRENCE_SQL, params![occurrence_id, occurrence.service_id, occurrence.period, occurrence.expected_amount, occurrence.paid_amount, occurrence.effective_date, occurrence.status, occurrence.transaction_id, occurrence.artifact_id, occurrence.source_reference, occurrence.raw_source, payload.context.actor_library_user_id, payload.context.source, timestamp]).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    sync_context(&payload.context, &app)?;
    let persisted =
        transaction_query_occurrence(&connection, &occurrence.service_id, &occurrence.period)?;
    drop(connection);
    Ok(persisted)
}

/// Marks the month of `saved` as paid for `service`: the expense that
/// paid it, with what was paid. The assistant and the forms both call it
/// after saving the expense.
pub(crate) fn record_service_payment(
    app: crate::host::AppHandle,
    context: FinanceContext,
    saved: &FinanceTransaction,
    service: &FinanceService,
) -> FinanceCommandResult<FinanceServiceOccurrence> {
    finance_save_service_occurrence(
        app,
        SaveFinanceServiceOccurrencePayload {
            context,
            occurrence: FinanceServiceOccurrence {
                id: uuid::Uuid::new_v4().to_string(),
                service_id: service.id.clone(),
                period: saved.effective_date[..7].to_string(),
                expected_amount: service.expected_amount.clone(),
                paid_amount: Some(saved.amount.clone()),
                effective_date: Some(saved.effective_date.clone()),
                status: "current".to_string(),
                transaction_id: Some(saved.id.clone()),
                artifact_id: None,
                source_reference: saved.source_reference.clone(),
                raw_source: saved.raw_source.clone(),
                actor_library_user_id: saved.actor_library_user_id.clone(),
                source: saved.source.clone(),
                created_at: None,
                updated_at: None,
                difference: None,
            },
            reason: None,
        },
    )
}

pub fn finance_list_all_service_occurrence_versions(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceServiceOccurrenceVersion>> {
    let connection = validate_context(&context, &app)?;
    let result = connection
        .prepare("SELECT id,occurrence_id,version_number,expected_amount,paid_amount,effective_date,status,transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,source,reason,created_at FROM finance_service_occurrence_versions ORDER BY created_at DESC")
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok(FinanceServiceOccurrenceVersion { id: row.get(0)?, occurrence_id: row.get(1)?, version_number: row.get(2)?, expected_amount: row.get(3)?, paid_amount: row.get(4)?, effective_date: row.get(5)?, status: row.get(6)?, transaction_id: row.get(7)?, artifact_id: row.get(8)?, source_reference: row.get(9)?, raw_source: row.get(10)?, actor_library_user_id: row.get(11)?, source: row.get(12)?, reason: row.get(13)?, created_at: row.get(14)? }))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string().into());
    result
}

pub fn finance_save_service_invoice(
    app: crate::host::AppHandle,
    payload: SaveFinanceServiceInvoicePayload,
) -> FinanceCommandResult<FinanceServiceInvoice> {
    let invoice = &payload.invoice;
    if invoice.id.trim().is_empty()
        || !valid_service_period(&invoice.period)
        || !valid_amount(&invoice.amount)
        || !matches!(invoice.currency.as_str(), "ARS" | "USD")
        || !matches!(
            invoice.validation_status.as_str(),
            "pending" | "valid" | "invalid" | "duplicate"
        )
        || invoice
            .service_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        || invoice
            .due_date
            .as_deref()
            .map(|value| !valid_iso_date(value))
            .unwrap_or(false)
        || invoice
            .provider
            .as_deref()
            .map(|value| !valid_bounded_text(value, 160, false))
            .unwrap_or(false)
        || invoice
            .source_reference
            .as_deref()
            .map(|value| !valid_bounded_text(value, 512, false))
            .unwrap_or(false)
        || invoice
            .raw_extraction
            .as_deref()
            .map(|value| !valid_bounded_text(value, 20_000, false))
            .unwrap_or(false)
    {
        return Err(
            "La factura requiere servicio, período, importe, moneda y datos válidos.".into(),
        );
    }
    let connection = validate_context(&payload.context, &app)?;
    let Some(service_id) = invoice.service_id.as_deref() else {
        return Err("La factura requiere un servicio válido.".into());
    };
    let (service_currency, service_provider): (String, Option<String>) = connection
        .query_row(
            "SELECT currency,provider FROM finance_services WHERE id=?1",
            [service_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "El servicio de la factura no existe.".to_string())?;
    if service_currency != invoice.currency {
        return Err("La moneda de la factura no coincide con el servicio.".into());
    }
    if let (Some(invoice_provider), Some(service_provider)) =
        (invoice.provider.as_deref(), service_provider.as_deref())
    {
        if normalize_service_text(invoice_provider) != normalize_service_text(service_provider) {
            return Err("El proveedor de la factura no coincide con el servicio.".into());
        }
    }
    if let Some(transaction_id) = invoice.transaction_id.as_deref() {
        let (transaction_currency, transaction_type, transaction_amount, linked_service): (String, String, String, Option<String>) = connection.query_row("SELECT currency,transaction_type,amount,service_id FROM finance_transactions WHERE id=?1 AND deleted_at IS NULL", [transaction_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).map_err(|_| "La transacción asociada a la factura no existe.".to_string())?;
        if transaction_currency != invoice.currency {
            return Err("La moneda de la factura no coincide con la transacción.".into());
        }
        if transaction_type != "expense" {
            return Err("La factura solo puede vincularse a un gasto.".into());
        }
        if transaction_amount != invoice.amount {
            return Err(
                "El importe de la factura debe coincidir exactamente con el gasto asociado.".into(),
            );
        }
        if linked_service
            .as_deref()
            .is_some_and(|linked| linked != service_id)
        {
            return Err("El gasto ya está vinculado a otro servicio.".into());
        }
    }
    if let Some(artifact_id) = invoice.artifact_id.as_deref() {
        let artifact_type: Option<String> = connection
            .query_row(
                "SELECT source_type FROM finance_source_artifacts WHERE id=?1",
                [artifact_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if artifact_type.as_deref() != Some("service_invoice") {
            return Err("El comprobante debe ser un artefacto de factura de servicio.".into());
        }
    }
    let mut database_transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let old_transaction: Option<String> = database_transaction
        .query_row(
            "SELECT transaction_id FROM finance_service_invoices WHERE id=?1",
            [&invoice.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    let timestamp = now();
    if let Some(old_transaction) = old_transaction.as_deref() {
        if invoice.transaction_id.as_deref() != Some(old_transaction) {
            database_transaction
                .execute(
                    "UPDATE finance_transactions SET service_id=NULL WHERE id=?1 AND service_id=?2",
                    params![old_transaction, service_id],
                )
                .map_err(|error| error.to_string())?;
            database_transaction.execute("UPDATE finance_purchases SET service_id=NULL,updated_at=?1 WHERE transaction_id=?2 AND service_id=?3", params![timestamp, old_transaction, service_id]).map_err(|error| error.to_string())?;
        }
    }
    database_transaction.execute("INSERT INTO finance_service_invoices(id,service_id,period,due_date,provider,amount,currency,transaction_id,artifact_id,validation_status,source_reference,raw_extraction,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13) ON CONFLICT(id) DO UPDATE SET service_id=excluded.service_id,period=excluded.period,due_date=excluded.due_date,provider=excluded.provider,amount=excluded.amount,currency=excluded.currency,transaction_id=excluded.transaction_id,artifact_id=excluded.artifact_id,validation_status=excluded.validation_status,source_reference=excluded.source_reference,raw_extraction=excluded.raw_extraction,updated_at=excluded.updated_at", params![invoice.id, service_id, invoice.period, invoice.due_date, invoice.provider, invoice.amount, invoice.currency, invoice.transaction_id, invoice.artifact_id, invoice.validation_status, invoice.source_reference, invoice.raw_extraction, timestamp]).map_err(|error| if error.to_string().contains("UNIQUE") { FinanceCommandError::from("El comprobante o la factura ya fue registrado.") } else { error.to_string().into() })?;
    if let Some(transaction_id) = invoice.transaction_id.as_deref() {
        database_transaction
            .execute(
                "UPDATE finance_transactions SET service_id=?1 WHERE id=?2 AND deleted_at IS NULL",
                params![service_id, transaction_id],
            )
            .map_err(|error| error.to_string())?;
        database_transaction
            .execute(
                "UPDATE finance_purchases SET service_id=?1,updated_at=?2 WHERE transaction_id=?3",
                params![service_id, timestamp, transaction_id],
            )
            .map_err(|error| error.to_string())?;
    }
    database_transaction
        .commit()
        .map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    let connection = validate_context(&payload.context, &app)?;
    let persisted = connection.query_row("SELECT id,service_id,period,due_date,provider,amount,currency,transaction_id,artifact_id,validation_status,source_reference,raw_extraction,created_at,updated_at FROM finance_service_invoices WHERE id=?1", [&invoice.id], |row| Ok(FinanceServiceInvoice { id: row.get(0)?, service_id: row.get(1)?, period: row.get(2)?, due_date: row.get(3)?, provider: row.get(4)?, amount: row.get(5)?, currency: row.get(6)?, transaction_id: row.get(7)?, artifact_id: row.get(8)?, validation_status: row.get(9)?, source_reference: row.get(10)?, raw_extraction: row.get(11)?, created_at: row.get(12)?, updated_at: row.get(13)? })).map_err(|error| error.to_string())?;
    Ok(persisted)
}

pub fn finance_list_service_invoices(
    app: crate::host::AppHandle,
    context: FinanceContext,
    period: Option<String>,
) -> FinanceCommandResult<Vec<FinanceServiceInvoice>> {
    if let Some(value) = period.as_deref() {
        if !valid_service_period(value) {
            return Err("El período debe tener formato YYYY-MM.".into());
        }
    }
    let connection = validate_context(&context, &app)?;
    let mut statement = connection.prepare("SELECT id,service_id,period,due_date,provider,amount,currency,transaction_id,artifact_id,validation_status,source_reference,raw_extraction,created_at,updated_at FROM finance_service_invoices WHERE (?1 IS NULL OR period=?1) ORDER BY period DESC,created_at DESC LIMIT 500").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![period], |row| {
            Ok(FinanceServiceInvoice {
                id: row.get(0)?,
                service_id: row.get(1)?,
                period: row.get(2)?,
                due_date: row.get(3)?,
                provider: row.get(4)?,
                amount: row.get(5)?,
                currency: row.get(6)?,
                transaction_id: row.get(7)?,
                artifact_id: row.get(8)?,
                validation_status: row.get(9)?,
                source_reference: row.get(10)?,
                raw_extraction: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string().into())
}

pub fn finance_get_dashboard(
    app: crate::host::AppHandle,
    context: FinanceContext,
    month: String,
) -> FinanceCommandResult<FinanceDashboard> {
    let connection = validate_context(&context, &app)?;
    let accounts = finance_list_accounts_inner(&connection)?;
    let categories = finance_list_categories_inner(&connection)?;
    let transaction_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM finance_transactions WHERE deleted_at IS NULL AND effective_date LIKE ?1 || '%'", [&month], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let savings_movement_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM finance_savings_movements WHERE effective_date LIKE ?1 || '%'",
            [&month],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(&format!(
            "{TRANSACTION_SELECT} WHERE t.deleted_at IS NULL AND t.effective_date LIKE ?1 || '%'
             ORDER BY t.effective_date DESC,t.created_at DESC LIMIT 500"
        ))
        .map_err(|e| e.to_string())?;
    let transactions = statement
        .query_map([&month], transaction_from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let mut income_by_currency = BTreeMap::new();
    let mut expense_by_currency = BTreeMap::new();
    for transaction in transactions
        .iter()
        .filter(|item| matches!(item.status.as_str(), "confirmed" | "corrected"))
    {
        if transaction.transaction_type == "income" {
            add_currency_total(
                &mut income_by_currency,
                &transaction.currency,
                &transaction.amount,
            );
        } else if transaction.transaction_type == "expense" {
            add_currency_total(
                &mut expense_by_currency,
                &transaction.currency,
                &transaction.amount,
            );
        }
    }
    let mut net_by_currency = BTreeMap::new();
    for currency in income_by_currency.keys().chain(expense_by_currency.keys()) {
        let income = income_by_currency
            .get(currency)
            .copied()
            .unwrap_or_default();
        let expense = expense_by_currency
            .get(currency)
            .copied()
            .unwrap_or_default();
        net_by_currency.insert(currency.clone(), format_cents(income - expense));
    }
    Ok(FinanceDashboard {
        accounts,
        categories,
        transactions,
        transactions_truncated: transaction_count > 500,
        income_total: income_by_currency
            .iter()
            .map(|(currency, value)| format!("{currency} {}", format_cents(*value)))
            .collect::<Vec<_>>()
            .join(" · "),
        expense_total: expense_by_currency
            .iter()
            .map(|(currency, value)| format!("{currency} {}", format_cents(*value)))
            .collect::<Vec<_>>()
            .join(" · "),
        net_total: net_by_currency
            .iter()
            .map(|(currency, value)| format!("{currency} {value}"))
            .collect::<Vec<_>>()
            .join(" · "),
        income_by_currency: income_by_currency
            .into_iter()
            .map(|(currency, value)| (currency, format_cents(value)))
            .collect(),
        expense_by_currency: expense_by_currency
            .into_iter()
            .map(|(currency, value)| (currency, format_cents(value)))
            .collect(),
        net_by_currency,
        debt_by_currency: formatted_totals(finance_debt_by_currency(&connection, &month)?),
        services_by_currency: formatted_totals(finance_services_paid_by_currency(&connection, &month)?),
        salary_by_currency: formatted_totals(finance_salary_by_currency(&connection, &month)?),
        debt_ratio_history: finance_debt_ratio_history(&connection, &month)?,
        savings: finance_list_savings_inner(&connection)?,
        savings_movements: finance_list_savings_movements_inner(&connection, &month)?,
        savings_movements_truncated: savings_movement_count > 500,
        merchants: finance_list_merchants_inner(&connection)?,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFinanceRecordPayload {
    pub context: FinanceContext,
    pub id: String,
}

pub fn finance_get_transaction(
    app: crate::host::AppHandle,
    payload: GetFinanceRecordPayload,
) -> FinanceCommandResult<FinanceTransaction> {
    if payload.id.trim().is_empty() {
        return Err("El movimiento requiere un identificador.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    load_transaction(&connection, &payload.id)?.ok_or_else(|| "El movimiento no existe.".into())
}

pub fn finance_list_all_transactions(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceTransaction>> {
    let connection = validate_context(&context, &app)?;
    let mut statement = connection
        .prepare(&format!(
            "{TRANSACTION_SELECT} WHERE t.deleted_at IS NULL ORDER BY t.effective_date DESC,t.created_at DESC LIMIT 5000"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], transaction_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub fn finance_list_all_savings_movements(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceSavingsMovement>> {
    let connection = validate_context(&context, &app)?;
    finance_list_savings_movements_all_inner(&connection).map_err(Into::into)
}

pub fn finance_list_accounts(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceAccount>> {
    let connection = validate_context(&context, &app)?;
    finance_list_accounts_inner(&connection).map_err(Into::into)
}

pub fn finance_list_categories(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<FinanceCategory>> {
    let connection = validate_context(&context, &app)?;
    finance_list_categories_inner(&connection).map_err(Into::into)
}

//// Net salary that pays a month's expenses: the one of the previous
/// period. A salary is collected at the end of its month (or the start of
/// the next) and what is paid during month M comes out of it.
fn finance_salary_by_currency(
    connection: &Connection,
    month: &str,
) -> Result<BTreeMap<String, i128>, String> {
    let Some(previous) = crate::finance_reconciliation::previous_period(month) else {
        return Ok(BTreeMap::new());
    };
    let mut statement = connection
        .prepare("SELECT currency,net_amount FROM finance_salary_receipts WHERE period=?1")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([previous], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut totals = BTreeMap::new();
    for row in rows {
        let (currency, amount) = row.map_err(|error| error.to_string())?;
        add_currency_total(&mut totals, &currency, &amount);
    }
    Ok(totals)
}

/// What was paid for credit cards in a month: the total of each statement,
/// counted in the month it is due (a loaded statement is a paid one).
fn finance_debt_by_currency(
    connection: &Connection,
    period: &str,
) -> Result<BTreeMap<String, i128>, String> {
    let mut totals = BTreeMap::new();
    let mut card_statement = connection
        .prepare("SELECT currency,total_due FROM finance_credit_card_statements WHERE substr(due_date,1,7)=?1")
        .map_err(|error| error.to_string())?;
    let card_rows = card_statement
        .query_map([period], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    for row in card_rows {
        let (currency, amount) = row.map_err(|error| error.to_string())?;
        add_currency_total(&mut totals, &currency, &amount);
    }
    Ok(totals)
}

/// Month a service payment was made: its expense's month (a card line
/// counts in its statement's due month), else the payment date, else the
/// service month.
pub(crate) const SERVICE_PAYMENT_MONTH: &str =
    "substr(COALESCE(t.effective_date,o.effective_date,o.period || '-01'),1,7)";

/// What was paid for services in a month, by the month of each payment.
fn finance_services_paid_by_currency(
    connection: &Connection,
    month: &str,
) -> Result<BTreeMap<String, i128>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT s.currency,o.paid_amount FROM finance_service_occurrences o
             JOIN finance_services s ON s.id=o.service_id
             LEFT JOIN finance_transactions t ON t.id=o.transaction_id AND t.deleted_at IS NULL
             WHERE o.paid_amount IS NOT NULL AND {SERVICE_PAYMENT_MONTH}=?1"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([month], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?;
    let mut totals = BTreeMap::new();
    for row in rows {
        let (currency, amount) = row.map_err(|error| error.to_string())?;
        add_currency_total(&mut totals, &currency, &amount);
    }
    Ok(totals)
}

fn formatted_totals(totals: BTreeMap<String, i128>) -> BTreeMap<String, String> {
    totals
        .into_iter()
        .map(|(currency, value)| (currency, format_cents(value)))
        .collect()
}

/// Cards and services paid per month against the salary that paid them,
/// for the months of the last year with a statement due or a service paid.
/// Months without either have no point: nothing was loaded for them.
fn finance_debt_ratio_history(
    connection: &Connection,
    end_period: &str,
) -> Result<Vec<FinanceDebtRatioHistoryPoint>, String> {
    let start_period = finance_history_start_period(end_period)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT substr(due_date,1,7) FROM finance_credit_card_statements
             WHERE substr(due_date,1,7) BETWEEN ?1 AND ?2
             UNION
             SELECT {SERVICE_PAYMENT_MONTH} FROM finance_service_occurrences o
             LEFT JOIN finance_transactions t ON t.id=o.transaction_id AND t.deleted_at IS NULL
             WHERE o.paid_amount IS NOT NULL AND {SERVICE_PAYMENT_MONTH} BETWEEN ?1 AND ?2
             ORDER BY 1"
        ))
        .map_err(|error| error.to_string())?;
    let periods = statement
        .query_map(params![start_period, end_period], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    periods
        .into_iter()
        .map(|period| {
            Ok(FinanceDebtRatioHistoryPoint {
                debt_by_currency: formatted_totals(finance_debt_by_currency(connection, &period)?),
                services_by_currency: formatted_totals(finance_services_paid_by_currency(connection, &period)?),
                salary_by_currency: formatted_totals(finance_salary_by_currency(connection, &period)?),
                period,
            })
        })
        .collect()
}

fn finance_history_start_period(end_period: &str) -> Result<String, String> {
    let (year, month) = end_period
        .split_once('-')
        .ok_or_else(|| "El período financiero es inválido.".to_string())?;
    let year = year
        .parse::<i32>()
        .map_err(|_| "El período financiero es inválido.".to_string())?;
    let month = month
        .parse::<i32>()
        .map_err(|_| "El período financiero es inválido.".to_string())?;
    if !(1..=12).contains(&month) {
        return Err("El período financiero es inválido.".into());
    }
    let months = year * 12 + month - 1 - 11;
    Ok(format!(
        "{:04}-{:02}",
        months.div_euclid(12),
        months.rem_euclid(12) + 1
    ))
}

pub fn finance_save_savings_reserve(
    app: crate::host::AppHandle,
    payload: SaveSavingsReservePayload,
) -> FinanceCommandResult<FinanceSavingsReserve> {
    let reserve = &payload.reserve;
    if reserve.id.trim().is_empty()
        || reserve.name.trim().is_empty()
        || !valid_amount(&reserve.opening_balance)
        || !matches!(reserve.currency.as_str(), "ARS" | "USD")
    {
        return Err("La reserva requiere nombre, moneda e importe válidos.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let timestamp = now();
    let ledger_account_id = format!("savings:{}", reserve.id);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO finance_savings_reserves (id,name,currency,opening_balance,objective,active,ledger_account_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8) ON CONFLICT(id) DO UPDATE SET name=excluded.name,currency=excluded.currency,opening_balance=excluded.opening_balance,objective=excluded.objective,active=excluded.active,ledger_account_id=COALESCE(finance_savings_reserves.ledger_account_id, excluded.ledger_account_id),updated_at=excluded.updated_at", params![reserve.id, reserve.name.trim(), reserve.currency, reserve.opening_balance, reserve.objective, reserve.active as i32, ledger_account_id, timestamp]).map_err(|error| error.to_string())?;
    let ledger_account_id: String = transaction
        .query_row(
            "SELECT ledger_account_id FROM finance_savings_reserves WHERE id=?1",
            [&reserve.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    // The internal ledger follows its reserve: same name, currency and state.
    transaction.execute("INSERT INTO finance_accounts (id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES (?1,?2,'savings_reserve',?3,'0',?4,?5,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name,currency=excluded.currency,active=excluded.active,updated_at=excluded.updated_at", params![ledger_account_id, reserve.name.trim(), reserve.currency, reserve.active as i32, timestamp]).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(reserve.clone())
}

fn reserve_balance(connection: &Connection, reserve_id: &str) -> Result<i128, String> {
    let opening: String = connection
        .query_row(
            "SELECT opening_balance FROM finance_savings_reserves WHERE id=?1",
            [reserve_id],
            |row| row.get(0),
        )
        .map_err(|_| "La reserva no existe.".to_string())?;
    let mut balance = parse_cents(&opening).unwrap_or_default();
    let mut statement = connection
        .prepare("SELECT movement_type,amount,status FROM finance_savings_movements WHERE reserve_id=?1")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([reserve_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (kind, amount, status) = row.map_err(|error| error.to_string())?;
        apply_savings_movement(&mut balance, &kind, &amount, &status);
    }
    Ok(balance)
}

pub fn finance_save_savings_movement(
    app: crate::host::AppHandle,
    payload: SaveSavingsMovementPayload,
) -> FinanceCommandResult<FinanceSavingsMovement> {
    let movement = &payload.movement;
    validate_savings_movement(movement)?;
    let connection = validate_context(&payload.context, &app)?;
    let reserve_currency: String = connection
        .query_row(
            "SELECT currency FROM finance_savings_reserves WHERE id = ?1 AND active = 1",
            [&movement.reserve_id],
            |row| row.get(0),
        )
        .map_err(|_| "La reserva no existe o está inactiva.".to_string())?;
    if reserve_currency != movement.currency {
        return Err("La moneda del movimiento no coincide con la reserva.".into());
    }
    let ledger_account_id: String = connection
        .query_row(
            "SELECT ledger_account_id FROM finance_savings_reserves WHERE id = ?1",
            [&movement.reserve_id],
            |row| row.get(0),
        )
        .map_err(|_| "La reserva no tiene cuenta contable interna.".to_string())?;
    if let Some(account_id) = movement.account_id.as_deref() {
        let linked: i64 = connection.query_row("SELECT COUNT(*) FROM finance_savings_accounts WHERE reserve_id = ?1 AND account_id = ?2", params![movement.reserve_id, account_id], |row| row.get(0)).map_err(|error| error.to_string())?;
        if linked == 0 {
            return Err("La cuenta no está vinculada a la reserva.".into());
        }
    }
    let counted = matches!(movement.status.as_str(), "confirmed" | "corrected");
    let moves_money = matches!(movement.movement_type.as_str(), "contribution" | "withdrawal");
    if counted && moves_money && movement.account_id.is_none() {
        return Err("Los aportes y retiros confirmados requieren una cuenta vinculada.".into());
    }
    let timestamp = now();
    let transfer_id = format!("savings-movement:{}", movement.id);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO finance_savings_movements (id,reserve_id,account_id,movement_type,amount,currency,effective_date,description,reason,source,status,actor_user_id,linked_transaction_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14) ON CONFLICT(id) DO UPDATE SET reserve_id=excluded.reserve_id,account_id=excluded.account_id,movement_type=excluded.movement_type,amount=excluded.amount,currency=excluded.currency,effective_date=excluded.effective_date,description=excluded.description,reason=excluded.reason,source=excluded.source,status=excluded.status,actor_user_id=excluded.actor_user_id,linked_transaction_id=excluded.linked_transaction_id,updated_at=excluded.updated_at", params![movement.id, movement.reserve_id, movement.account_id, movement.movement_type, movement.amount, movement.currency, movement.effective_date, movement.description, movement.reason, movement.source, movement.status, movement.actor_user_id, movement.linked_transaction_id, timestamp]).map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE finance_savings_movements SET actor_library_user_id=?1 WHERE id=?2",
            params![movement.actor_library_user_id, movement.id],
        )
        .map_err(|error| error.to_string())?;
    // The transfer between the account and the reserve's ledger follows the
    // movement: it exists only while the movement counts and moves money.
    match movement.account_id.as_deref().filter(|_| counted && moves_money) {
        Some(account_id) => {
            let (source_account, destination_account) = if movement.movement_type == "contribution" {
                (account_id, ledger_account_id.as_str())
            } else {
                (ledger_account_id.as_str(), account_id)
            };
            transaction.execute("INSERT INTO finance_transactions (id,transaction_type,amount,currency,effective_date,account_id,destination_account_id,description,source,status,actor_user_id,created_at,updated_at) VALUES (?1,'transfer',?2,?3,?4,?5,?6,?7,'savings',?8,?9,?10,?10) ON CONFLICT(id) DO UPDATE SET amount=excluded.amount,effective_date=excluded.effective_date,account_id=excluded.account_id,destination_account_id=excluded.destination_account_id,description=excluded.description,status=excluded.status,actor_user_id=excluded.actor_user_id,deleted_at=NULL,updated_at=excluded.updated_at", params![transfer_id, movement.amount, movement.currency, movement.effective_date, source_account, destination_account, movement.description, movement.status, movement.actor_user_id, timestamp]).map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE finance_transactions SET actor_library_user_id=?1 WHERE id=?2",
                    params![movement.actor_library_user_id, transfer_id],
                )
                .map_err(|error| error.to_string())?;
        }
        None => {
            transaction
                .execute(
                    "UPDATE finance_transactions SET deleted_at=?1,updated_at=?1 WHERE id=?2 AND deleted_at IS NULL",
                    params![timestamp, transfer_id],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(movement.clone())
}

/// Buys a currency for a reserve (money leaves the account and enters the
/// reserve) or sells savings (money leaves the reserve and enters the
/// account). Neither is income nor expense.
pub fn finance_save_savings_exchange(
    app: crate::host::AppHandle,
    payload: SaveSavingsExchangePayload,
) -> FinanceCommandResult<FinanceSavedSavingsExchange> {
    let exchange = &payload.exchange;
    let actor_library_user_id = payload.context.actor_library_user_id.clone();
    let selling = match exchange.direction.as_deref().unwrap_or("buy") {
        "buy" => false,
        "sell" => true,
        _ => return Err("La dirección del cambio debe ser buy o sell.".into()),
    };
    if exchange.id.trim().is_empty()
        || exchange.reserve_id.trim().is_empty()
        || exchange.source_account_id.trim().is_empty()
        || exchange.description.trim().is_empty()
        || !valid_amount(&exchange.source_amount)
        || !valid_amount(&exchange.savings_amount)
        || !valid_iso_date(exchange.effective_date.get(..10).unwrap_or_default())
        || !matches!(exchange.source_currency.as_str(), "ARS" | "USD")
        || !matches!(exchange.savings_currency.as_str(), "ARS" | "USD")
        || exchange.source_currency == exchange.savings_currency
    {
        return Err(
            "El cambio de moneda requiere importes, monedas distintas, fecha y cuentas válidos.".into(),
        );
    }
    let connection = validate_context(&payload.context, &app)?;
    let (reserve_currency, reserve_name, ledger_account_id): (String, String, String) = connection
        .query_row(
            "SELECT currency,name,ledger_account_id FROM finance_savings_reserves WHERE id = ?1 AND active = 1",
            [&exchange.reserve_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "La reserva no existe o está inactiva.".to_string())?;
    if reserve_currency != exchange.savings_currency {
        return Err("La moneda del ahorro no coincide con la reserva.".into());
    }
    let account_currency: String = connection
        .query_row(
            "SELECT currency FROM finance_accounts WHERE id = ?1 AND active = 1 AND account_type <> 'savings_reserve'",
            [&exchange.source_account_id],
            |row| row.get(0),
        )
        .map_err(|_| "La cuenta no existe, está inactiva o no es una cuenta de pago.".to_string())?;
    if account_currency != exchange.source_currency {
        return Err("La moneda de la cuenta no coincide con la del cambio.".into());
    }
    let movement_id = format!("savings-exchange:{}", exchange.id);
    if selling {
        let available = reserve_balance(&connection, &exchange.reserve_id)?;
        // A sale being corrected already took its amount out of the balance.
        let already_sold = connection
            .query_row(
                "SELECT amount FROM finance_savings_movements
                 WHERE id=?1 AND movement_type='withdrawal' AND status IN ('confirmed','corrected')",
                [&movement_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .and_then(|amount| parse_cents(&amount).ok())
            .unwrap_or_default();
        let requested = parse_cents(&exchange.savings_amount).unwrap_or_default();
        if requested > available + already_sold {
            return Err(format!(
                "La reserva {reserve_name} tiene {reserve_currency} {}; no alcanza para vender {reserve_currency} {}.",
                format_cents(available + already_sold),
                exchange.savings_amount
            )
            .into());
        }
    }

    let timestamp = now();
    let (account_id, destination_account_id) = if selling {
        (ledger_account_id.as_str(), exchange.source_account_id.as_str())
    } else {
        (exchange.source_account_id.as_str(), ledger_account_id.as_str())
    };
    let (movement_type, reason) = if selling {
        ("withdrawal", Some(format!("Venta de {}", exchange.savings_currency)))
    } else {
        ("contribution", None)
    };
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let source_artifact_id = exchange
        .source_reference
        .as_deref()
        .filter(|reference| !reference.trim().is_empty())
        .map(|_| format!("savings-exchange-source:{}", exchange.id));
    if let (Some(artifact_id), Some(reference)) = (
        source_artifact_id.as_deref(),
        exchange.source_reference.as_deref(),
    ) {
        transaction.execute("INSERT INTO finance_source_artifacts(id,source_type,reference,raw_text,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET reference=excluded.reference,raw_text=excluded.raw_text", params![artifact_id, payload.context.source, reference, exchange.raw_source, timestamp]).map_err(|error| error.to_string())?;
    }
    transaction.execute("INSERT INTO finance_transactions (id,transaction_type,amount,currency,effective_date,account_id,destination_account_id,category_id,description,source,status,actor_user_id,source_artifact_id,actor_library_user_id,created_at,updated_at) VALUES (?1,'exchange',?2,?3,?4,?5,?6,NULL,?7,'savings_exchange','confirmed',?8,?9,?10,?11,?11) ON CONFLICT(id) DO UPDATE SET transaction_type='exchange',amount=excluded.amount,currency=excluded.currency,effective_date=excluded.effective_date,account_id=excluded.account_id,destination_account_id=excluded.destination_account_id,description=excluded.description,status=excluded.status,actor_user_id=excluded.actor_user_id,source_artifact_id=excluded.source_artifact_id,actor_library_user_id=excluded.actor_library_user_id,deleted_at=NULL,updated_at=excluded.updated_at", params![exchange.id, exchange.source_amount, exchange.source_currency, exchange.effective_date, account_id, destination_account_id, exchange.description, exchange.actor_user_id, source_artifact_id, actor_library_user_id, timestamp]).map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO finance_savings_movements (id,reserve_id,account_id,movement_type,amount,currency,effective_date,description,reason,source,status,actor_user_id,linked_transaction_id,actor_library_user_id,created_at,updated_at) VALUES (?1,?2,NULL,?3,?4,?5,?6,?7,?8,'savings_exchange','confirmed',?9,?10,?11,?12,?12) ON CONFLICT(id) DO UPDATE SET reserve_id=excluded.reserve_id,movement_type=excluded.movement_type,amount=excluded.amount,currency=excluded.currency,effective_date=excluded.effective_date,description=excluded.description,reason=excluded.reason,status=excluded.status,actor_user_id=excluded.actor_user_id,linked_transaction_id=excluded.linked_transaction_id,actor_library_user_id=excluded.actor_library_user_id,updated_at=excluded.updated_at", params![movement_id, exchange.reserve_id, movement_type, exchange.savings_amount, exchange.savings_currency, exchange.effective_date, exchange.description, reason, exchange.actor_user_id, exchange.id, actor_library_user_id, timestamp]).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    let saved_transaction = load_transaction(&connection, &exchange.id)?
        .ok_or_else(|| "El cambio de moneda no quedó guardado.".to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(FinanceSavedSavingsExchange {
        movement: FinanceSavingsMovement {
            id: movement_id,
            reserve_id: exchange.reserve_id.clone(),
            account_id: None,
            movement_type: movement_type.into(),
            amount: exchange.savings_amount.clone(),
            currency: exchange.savings_currency.clone(),
            effective_date: exchange.effective_date.clone(),
            description: exchange.description.clone(),
            reason,
            source: "savings_exchange".into(),
            status: "confirmed".into(),
            actor_user_id: exchange.actor_user_id,
            actor_library_user_id: Some(actor_library_user_id),
            linked_transaction_id: Some(exchange.id.clone()),
        },
        transaction: saved_transaction,
    })
}

fn validate_savings_movement(movement: &FinanceSavingsMovement) -> Result<(), String> {
    if !valid_amount(&movement.amount)
        || movement.effective_date.len() < 10
        || !matches!(
            movement.movement_type.as_str(),
            "contribution" | "withdrawal" | "return" | "loss" | "adjustment"
        )
        || !matches!(movement.currency.as_str(), "ARS" | "USD")
        || (movement.movement_type == "withdrawal"
            && movement.reason.as_deref().unwrap_or("").trim().is_empty())
    {
        return Err(
            "El movimiento de ahorro requiere datos válidos y motivo para retiros.".to_string(),
        );
    }
    Ok(())
}

pub fn finance_link_savings_account(
    app: crate::host::AppHandle,
    payload: LinkSavingsAccountPayload,
) -> FinanceCommandResult<()> {
    if payload.reserve_id.trim().is_empty() || payload.account_id.trim().is_empty() {
        return Err("La reserva y la cuenta son obligatorias.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let reserve_currency: String = connection
        .query_row(
            "SELECT currency FROM finance_savings_reserves WHERE id = ?1 AND active = 1",
            [&payload.reserve_id],
            |row| row.get(0),
        )
        .map_err(|_| "La reserva no existe o está inactiva.".to_string())?;
    let account_currency: String = connection
        .query_row(
            "SELECT currency FROM finance_accounts WHERE id = ?1 AND active = 1",
            [&payload.account_id],
            |row| row.get(0),
        )
        .map_err(|_| "La cuenta no existe o está inactiva.".to_string())?;
    if reserve_currency != account_currency {
        return Err("La reserva y la cuenta deben usar la misma moneda.".into());
    }
    connection.execute("INSERT OR IGNORE INTO finance_savings_accounts (reserve_id,account_id,created_at) VALUES (?1,?2,?3)", params![payload.reserve_id, payload.account_id, now()]).map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(())
}

pub fn finance_save_account(
    app: crate::host::AppHandle,
    payload: SaveAccountPayload,
) -> FinanceCommandResult<FinanceAccount> {
    if payload.account.name.trim().is_empty()
        || !matches!(payload.account.currency.as_str(), "ARS" | "USD")
    {
        return Err("La cuenta requiere nombre y moneda válidos.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let timestamp = now();
    connection.execute("INSERT INTO finance_accounts (id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES (?1,?2,?3,?4,'0',?5,?6,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name, account_type=excluded.account_type, currency=excluded.currency, active=excluded.active, updated_at=excluded.updated_at", params![payload.account.id, payload.account.name.trim(), payload.account.account_type, payload.account.currency, payload.account.active as i32, timestamp]).map_err(|e| e.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(payload.account)
}

pub fn finance_save_category(
    app: crate::host::AppHandle,
    payload: SaveCategoryPayload,
) -> FinanceCommandResult<FinanceCategory> {
    if payload.category.name.trim().is_empty()
        || !matches!(payload.category.kind.as_str(), "income" | "expense")
    {
        return Err("La categoría requiere nombre.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let timestamp = now();
    if payload.category.parent_id.as_deref() == Some(payload.category.id.as_str()) {
        return Err("Una categoría no puede ser su propia categoría padre.".into());
    }
    if let Some(parent_id) = payload.category.parent_id.as_deref() {
        let parent_kind: String = connection
            .query_row(
                "SELECT kind FROM finance_categories WHERE id=?1 AND active=1",
                [parent_id],
                |row| row.get(0),
            )
            .map_err(|_| "La categoría padre no existe o está inactiva.".to_string())?;
        if parent_kind != payload.category.kind {
            return Err("La categoría padre debe tener el mismo tipo.".into());
        }
    }
    connection.execute("INSERT INTO finance_categories (id,name,kind,parent_id,active,description,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind, parent_id=excluded.parent_id, active=excluded.active, description=excluded.description, updated_at=excluded.updated_at", params![payload.category.id, payload.category.name.trim(), payload.category.kind, payload.category.parent_id, payload.category.active as i32, payload.category.description, timestamp]).map_err(|e| e.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(payload.category)
}

pub fn finance_delete_transaction(
    app: crate::host::AppHandle,
    payload: DeleteFinanceEntityPayload,
) -> FinanceCommandResult<()> {
    if payload.id.trim().is_empty() {
        return Err("El movimiento es obligatorio.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    if let Some(owner) = transaction_owner(&connection, &payload.id)? {
        return Err(format!(
            "El movimiento pertenece a un {owner}; eliminá ese documento en lugar del movimiento."
        )
        .into());
    }
    let changed = connection.execute("UPDATE finance_transactions SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL", params![now(), payload.id]).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("El movimiento no existe o ya fue eliminado.".into());
    }
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(())
}

pub fn finance_delete_account(
    app: crate::host::AppHandle,
    payload: DeleteFinanceEntityPayload,
) -> FinanceCommandResult<()> {
    if payload.id.trim().is_empty() {
        return Err("La cuenta es obligatoria.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let changed = connection
        .execute(
            "UPDATE finance_accounts SET active = 0, updated_at = ?1 WHERE id = ?2 AND active = 1",
            params![now(), payload.id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("La cuenta no existe o ya está inactiva.".into());
    }
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(())
}

pub fn finance_delete_category(
    app: crate::host::AppHandle,
    payload: DeleteFinanceEntityPayload,
) -> FinanceCommandResult<()> {
    if payload.id.trim().is_empty() {
        return Err("La categoría es obligatoria.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let changed = connection.execute("UPDATE finance_categories SET active = 0, updated_at = ?1 WHERE id = ?2 AND active = 1", params![now(), payload.id]).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("La categoría no existe o ya está inactiva.".into());
    }
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(())
}

fn clear_finance_data(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute_batch(
            "DELETE FROM finance_link_log;
             DELETE FROM finance_review_items;
             DELETE FROM finance_applied_requests;
             DELETE FROM finance_budget_alerts;
             DELETE FROM finance_budgets;
             DELETE FROM finance_loan_installments;
             DELETE FROM finance_loans;
             DELETE FROM finance_merchant_aliases;
             DELETE FROM finance_product_aliases;
             DELETE FROM finance_service_occurrence_versions;
             DELETE FROM finance_service_occurrences;
             DELETE FROM finance_service_invoices;
             DELETE FROM finance_price_observations;
             DELETE FROM finance_extraction_results;
             DELETE FROM finance_salary_concepts;
             DELETE FROM finance_purchase_items;
             DELETE FROM finance_credit_card_statement_items;
             DELETE FROM finance_installments;
             DELETE FROM finance_savings_accounts;
             DELETE FROM finance_savings_movements;
             DELETE FROM finance_salary_receipts;
             DELETE FROM finance_purchases;
             DELETE FROM finance_credit_card_statements;
             DELETE FROM finance_receipts;
             DELETE FROM finance_installment_plans;
             DELETE FROM finance_transactions;
             DELETE FROM finance_source_artifacts;
             DELETE FROM finance_products;
             DELETE FROM finance_merchants;
             DELETE FROM finance_services;
             DELETE FROM finance_savings_reserves;
             DELETE FROM finance_categories;
             DELETE FROM finance_accounts;",
        )
        .map_err(|error| error.to_string())?;
    crate::database::ensure_default_finance_categories(&transaction)
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn finance_clear_all_data(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<()> {
    let mut connection = validate_context(&context, &app)?;
    clear_finance_data(&mut connection)?;
    drop(connection);
    sync_context(&context, &app)?;
    Ok(())
}

/// The document a movement belongs to, when it is not a loose one. Only
/// that document changes its amount, date, account or state.
pub(crate) fn transaction_owner(
    connection: &Connection,
    transaction_id: &str,
) -> Result<Option<&'static str>, String> {
    const OWNERS: [(&str, &str); 6] = [
        (
            "SELECT EXISTS(SELECT 1 FROM finance_credit_card_statement_items WHERE transaction_id=?1)",
            "resumen de tarjeta",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM finance_purchases WHERE transaction_id=?1)",
            "ticket",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM finance_salary_receipts WHERE transaction_id=?1)",
            "recibo de sueldo",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM finance_savings_movements
                           WHERE linked_transaction_id=?1 OR 'savings-movement:' || id=?1)",
            "movimiento de ahorro",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM finance_installments WHERE transaction_id=?1)",
            "plan de cuotas",
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM finance_loan_installments WHERE transaction_id=?1)",
            "préstamo",
        ),
    ];
    for (query, owner) in OWNERS {
        let owned: bool = connection
            .query_row(query, [transaction_id], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if owned {
            return Ok(Some(owner));
        }
    }
    Ok(None)
}

pub fn finance_save_transaction(
    app: crate::host::AppHandle,
    payload: SaveTransactionPayload,
) -> FinanceCommandResult<FinanceTransaction> {
    finance_save_transaction_linked(app, payload).map(|(transaction, _)| transaction)
}

/// Saves a loose movement. An expense paid with a credit card waits for the
/// statement that pays it (`card_unpaid`) and joins its line when that
/// statement is already loaded. A movement that belongs to a document only
/// takes a new category, description or service.
pub fn finance_save_transaction_linked(
    app: crate::host::AppHandle,
    payload: SaveTransactionPayload,
) -> FinanceCommandResult<(FinanceTransaction, LinkOutcome)> {
    let transaction = &payload.transaction;
    if transaction.transaction_type == "exchange" {
        return Err("Los cambios de moneda se registran con el ahorro, no como movimiento suelto.".into());
    }
    if transaction.id.trim().is_empty()
        || !valid_amount(&transaction.amount)
        || !valid_iso_date(&transaction.effective_date)
        || transaction.account_id.trim().is_empty()
        || transaction.account_id.len() > 160
        || !valid_bounded_text(&transaction.description, 1_000, true)
        || transaction
            .source_reference
            .as_deref()
            .map(|value| !valid_bounded_text(value, 512, false))
            .unwrap_or(false)
        || transaction
            .raw_source
            .as_deref()
            .map(|value| !valid_bounded_text(value, 20_000, false))
            .unwrap_or(false)
        || !matches!(
            transaction.transaction_type.as_str(),
            "income" | "expense" | "transfer" | "adjustment"
        )
        || !matches!(transaction.currency.as_str(), "ARS" | "USD")
        || !matches!(
            transaction.status.as_str(),
            "pending" | "confirmed" | "corrected" | "discarded" | CARD_UNPAID
        )
    {
        return Err("El movimiento requiere importe, fecha y cuenta válidos.".into());
    }
    let connection = validate_context(&payload.context, &app)?;
    let existing = load_transaction(&connection, &transaction.id)?;
    let owner = match existing.as_ref() {
        Some(existing) => transaction_owner(&connection, &existing.id)?,
        None => None,
    };
    if let (Some(owner), Some(existing)) = (owner, existing.as_ref()) {
        let unchanged = existing.transaction_type == transaction.transaction_type
            && parse_cents(&existing.amount) == parse_cents(&transaction.amount)
            && existing.currency == transaction.currency
            && existing.effective_date.get(..10) == transaction.effective_date.get(..10)
            && existing.account_id == transaction.account_id
            && existing.destination_account_id == transaction.destination_account_id
            && existing.status == transaction.status;
        if !unchanged {
            return Err(format!(
                "Este movimiento pertenece a un {owner}: el importe, la fecha, la cuenta y el estado se corrigen desde ese documento. Acá solo cambian la categoría, la descripción y el servicio."
            )
            .into());
        }
    }
    let (account_type, account_currency): (String, String) = connection
        .query_row(
            "SELECT account_type,currency FROM finance_accounts WHERE id = ?1 AND active = 1",
            [&transaction.account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "La cuenta de origen no existe o está inactiva.".to_string())?;
    // Credit cards take pesos and dollars; every other account has one currency.
    let credit_card = account_type == "credit_card";
    if !credit_card && account_currency != transaction.currency {
        return Err("La moneda del movimiento no coincide con la cuenta.".into());
    }
    if let Some(category_id) = transaction
        .category_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let category_kind: String = connection
            .query_row(
                "SELECT kind FROM finance_categories WHERE id=?1 AND active=1",
                [category_id],
                |row| row.get(0),
            )
            .map_err(|_| "La categoría no existe o está inactiva.".to_string())?;
        if matches!(transaction.transaction_type.as_str(), "income" | "expense")
            && category_kind != transaction.transaction_type
        {
            return Err("La categoría no coincide con el tipo de movimiento.".into());
        }
    }
    if let Some(service_id) = transaction
        .service_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let service_currency: String = connection
            .query_row(
                "SELECT currency FROM finance_services WHERE id=?1 AND active=1",
                [service_id],
                |row| row.get(0),
            )
            .map_err(|_| "El servicio no existe o está inactivo.".to_string())?;
        if transaction.transaction_type != "expense" || service_currency != transaction.currency {
            return Err("El servicio solo puede asociarse a un gasto de su misma moneda.".into());
        }
    }
    if transaction.transaction_type == "transfer" {
        let destination_id = transaction
            .destination_account_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Una transferencia requiere cuenta destino.".to_string())?;
        if destination_id == transaction.account_id {
            return Err("La transferencia debe tener cuentas diferentes.".into());
        }
        let destination_currency: String = connection
            .query_row(
                "SELECT currency FROM finance_accounts WHERE id = ?1 AND active = 1",
                [destination_id],
                |row| row.get(0),
            )
            .map_err(|_| "La cuenta destino no existe o está inactiva.".to_string())?;
        if destination_currency != transaction.currency {
            return Err("Las cuentas de una transferencia deben usar la misma moneda.".into());
        }
    }
    let timestamp = now();
    let database_transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let mut outcome = LinkOutcome::default();
    if owner.is_some() {
        database_transaction
            .execute(
                "UPDATE finance_transactions SET category_id=?1,description=?2,service_id=?3,updated_at=?4 WHERE id=?5",
                params![transaction.category_id, transaction.description, transaction.service_id, timestamp, transaction.id],
            )
            .map_err(|error| error.to_string())?;
    } else {
        let waits_for_statement = credit_card
            && transaction.transaction_type == "expense"
            && existing.as_ref().map_or(true, |existing| existing.status == CARD_UNPAID)
            && matches!(transaction.status.as_str(), "confirmed" | "corrected" | CARD_UNPAID);
        let status = if waits_for_statement { CARD_UNPAID } else { transaction.status.as_str() };
        let purchase_date = waits_for_statement.then(|| transaction.effective_date[..10].to_string());
        let mut source_artifact_id = transaction.source_artifact_id.clone();
        if let Some(reference) = transaction
            .source_reference
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let artifact_id = source_artifact_id
                .get_or_insert_with(|| format!("transaction-source:{}", transaction.id));
            database_transaction.execute("INSERT INTO finance_source_artifacts(id,source_type,reference,raw_text,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET reference=excluded.reference,raw_text=excluded.raw_text", params![artifact_id.as_str(), payload.context.source, reference, transaction.raw_source, timestamp]).map_err(|error| error.to_string())?;
        }
        // An update keeps the channel the movement was created from.
        database_transaction.execute("INSERT INTO finance_transactions (id,transaction_type,amount,currency,effective_date,purchase_date,account_id,destination_account_id,category_id,description,source,status,actor_user_id,source_artifact_id,service_id,merchant_id,operation_fingerprint,installment_id,source_reference,raw_source,actor_library_user_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?22) ON CONFLICT(id) DO UPDATE SET transaction_type=excluded.transaction_type,amount=excluded.amount,currency=excluded.currency,effective_date=excluded.effective_date,purchase_date=excluded.purchase_date,account_id=excluded.account_id,destination_account_id=excluded.destination_account_id,category_id=excluded.category_id,description=excluded.description,status=excluded.status,actor_user_id=excluded.actor_user_id,source_artifact_id=excluded.source_artifact_id,service_id=excluded.service_id,merchant_id=excluded.merchant_id,operation_fingerprint=COALESCE(excluded.operation_fingerprint,finance_transactions.operation_fingerprint),installment_id=COALESCE(excluded.installment_id,finance_transactions.installment_id),source_reference=excluded.source_reference,raw_source=excluded.raw_source,actor_library_user_id=excluded.actor_library_user_id,deleted_at=NULL,updated_at=excluded.updated_at", params![transaction.id, transaction.transaction_type, transaction.amount, transaction.currency, transaction.effective_date, purchase_date, transaction.account_id, transaction.destination_account_id, transaction.category_id, transaction.description, payload.context.source, status, transaction.actor_user_id, source_artifact_id, transaction.service_id, transaction.merchant_id, transaction.operation_fingerprint, transaction.installment_id, transaction.source_reference, transaction.raw_source, payload.context.actor_library_user_id, timestamp]).map_err(|e| e.to_string())?;
        if waits_for_statement {
            crate::finance_matching::link_unpaid_expense(&database_transaction, &transaction.id, &mut outcome)?;
        }
    }
    database_transaction
        .commit()
        .map_err(|error| error.to_string())?;
    // A card expense joined to its statement line lives on as that line.
    let saved = match load_transaction(&connection, &transaction.id)? {
        Some(saved) => saved,
        None => {
            let line_transaction_id: Option<String> = connection
                .query_row(
                    "SELECT i.transaction_id FROM finance_link_log l
                     JOIN finance_credit_card_statement_items i ON i.id=l.target_id
                     WHERE l.subject_id=?1 AND l.undone_at IS NULL ORDER BY l.created_at DESC LIMIT 1",
                    [&transaction.id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .flatten();
            match line_transaction_id {
                Some(id) => load_transaction(&connection, &id)?.unwrap_or_else(|| transaction.clone()),
                None => transaction.clone(),
            }
        }
    };
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok((saved, outcome))
}

fn finance_list_accounts_inner(connection: &Connection) -> Result<Vec<FinanceAccount>, String> {
    let mut statement = connection.prepare("SELECT id,name,account_type,currency,active FROM finance_accounts WHERE account_type <> 'savings_reserve' ORDER BY name").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(FinanceAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                account_type: row.get(2)?,
                currency: row.get(3)?,
                active: row.get::<_, i32>(4)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub(crate) fn finance_list_savings_inner(
    connection: &Connection,
) -> Result<Vec<FinanceSavingsReserve>, String> {
    let mut statement = connection.prepare("SELECT id,name,currency,opening_balance,objective,active FROM finance_savings_reserves ORDER BY name").map_err(|error| error.to_string())?;
    let reserves = statement
        .query_map([], |row| {
            Ok(FinanceSavingsReserve {
                id: row.get(0)?,
                name: row.get(1)?,
                currency: row.get(2)?,
                opening_balance: row.get(3)?,
                objective: row.get(4)?,
                active: row.get::<_, i32>(5)? != 0,
                balance: "0".to_string(),
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    reserves.into_iter().map(|mut reserve| {
        let mut balance = parse_cents(&reserve.opening_balance).unwrap_or_default();
        let mut movements = connection.prepare("SELECT movement_type,amount,status FROM finance_savings_movements WHERE reserve_id = ?1").map_err(|error| error.to_string())?;
        let rows = movements.query_map([&reserve.id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))).map_err(|error| error.to_string())?;
        for row in rows {
            let (kind, amount, status) = row.map_err(|error| error.to_string())?;
            apply_savings_movement(&mut balance, &kind, &amount, &status);
        }
        reserve.balance = format_cents(balance);
        Ok(reserve)
    }).collect()
}

fn finance_list_savings_movements_inner(
    connection: &Connection,
    month: &str,
) -> Result<Vec<FinanceSavingsMovement>, String> {
    let mut statement=connection.prepare("SELECT id,reserve_id,account_id,movement_type,amount,currency,effective_date,description,reason,source,status,actor_user_id,linked_transaction_id,actor_library_user_id FROM finance_savings_movements WHERE effective_date LIKE ?1 || '%' ORDER BY effective_date DESC,created_at DESC LIMIT 500").map_err(|error|error.to_string())?;
    let movements = statement
        .query_map([month], |row| {
            Ok(FinanceSavingsMovement {
                id: row.get(0)?,
                reserve_id: row.get(1)?,
                account_id: row.get(2)?,
                movement_type: row.get(3)?,
                amount: row.get(4)?,
                currency: row.get(5)?,
                effective_date: row.get(6)?,
                description: row.get(7)?,
                reason: row.get(8)?,
                source: row.get(9)?,
                status: row.get(10)?,
                actor_user_id: row.get(11)?,
                actor_library_user_id: row.get(13)?,
                linked_transaction_id: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(movements)
}

fn finance_list_savings_movements_all_inner(
    connection: &Connection,
) -> Result<Vec<FinanceSavingsMovement>, String> {
    let mut statement = connection
        .prepare("SELECT id,reserve_id,account_id,movement_type,amount,currency,effective_date,description,reason,source,status,actor_user_id,linked_transaction_id,actor_library_user_id FROM finance_savings_movements ORDER BY effective_date DESC,created_at DESC LIMIT 5000")
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map([], |row| {
            Ok(FinanceSavingsMovement {
                id: row.get(0)?,
                reserve_id: row.get(1)?,
                account_id: row.get(2)?,
                movement_type: row.get(3)?,
                amount: row.get(4)?,
                currency: row.get(5)?,
                effective_date: row.get(6)?,
                description: row.get(7)?,
                reason: row.get(8)?,
                source: row.get(9)?,
                status: row.get(10)?,
                actor_user_id: row.get(11)?,
                linked_transaction_id: row.get(12)?,
                actor_library_user_id: row.get(13)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

fn finance_list_merchants_inner(connection: &Connection) -> Result<Vec<FinanceMerchant>, String> {
    connection
        .prepare("SELECT id,name FROM finance_merchants ORDER BY name")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(FinanceMerchant {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn finance_list_categories_inner(connection: &Connection) -> Result<Vec<FinanceCategory>, String> {
    connection
        .prepare("SELECT id,name,kind,active,parent_id,description FROM finance_categories ORDER BY name")
        .map_err(|e| e.to_string())?
        .query_map([], |row| {
            Ok(FinanceCategory {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                active: row.get::<_, i32>(3)? != 0,
                parent_id: row.get(4)?,
                description: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_savings_movement, clear_finance_data, finance_dev_query, finance_list_accounts_inner,
        format_cents, parse_cents, seed_finance_demo_data, transaction_owner, valid_amount,
        validate_finance_dev_sql, validate_savings_movement, FinanceCommandError,
        FinanceSavingsMovement, FINANCE_DEV_TABLES,
    };
    use rusqlite::{params, Connection};

    #[test]
    fn accepts_exact_decimal_amounts_without_float_conversion() {
        assert!(valid_amount("1500.25"));
        assert!(valid_amount("0"));
        assert!(!valid_amount("1,500"));
        assert!(!valid_amount(""));
    }

    #[test]
    fn keeps_decimal_values_in_minor_units() {
        assert_eq!(parse_cents("1500.25"), Ok(150025));
        assert_eq!(parse_cents("-0.50"), Err(()));
        assert_eq!(parse_cents("12.345"), Err(()));
        assert_eq!(format_cents(-50), "-0.50");
    }

    #[test]
    fn rejects_invalid_calendar_dates_without_panicking() {
        assert!(super::valid_iso_date("2024-02-29"));
        assert!(!super::valid_iso_date("2023-02-29"));
        assert!(!super::valid_iso_date("2026-04-31"));
        assert!(!super::valid_iso_date("2026-1-01"));
        assert!(!super::valid_iso_date("á2026-01-01"));
    }

    #[test]
    fn payment_accounts_are_references_and_hide_internal_savings_ledgers() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection.execute("INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES('account','Digital','bank','ARS','100.00',1,'now','now')", []).expect("account fixture");
        for (id, kind, amount, date) in [
            ("old-salary", "income", "900.00", "2026-07-31"),
            ("current-salary", "income", "200.00", "2026-08-01"),
            ("current-expense", "expense", "50.00", "2026-08-02"),
        ] {
            connection.execute("INSERT INTO finance_transactions(id,transaction_type,amount,currency,effective_date,account_id,description,source,status,created_at,updated_at) VALUES(?1,?2,?3,'ARS',?4,'account','Prueba','manual','confirmed','now','now')", params![id, kind, amount, date]).expect("transaction fixture");
        }

        connection.execute("INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES('savings:reserve','Reserva','savings_reserve','ARS','0',1,'now','now')", []).expect("savings ledger fixture");

        let accounts =
            finance_list_accounts_inner(&connection).expect("payment account references");

        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "account");
        assert_eq!(
            serde_json::to_value(&accounts[0]).expect("serialize account reference"),
            serde_json::json!({
                "id": "account",
                "name": "Digital",
                "accountType": "bank",
                "currency": "ARS",
                "active": true
            })
        );
    }

    #[test]
    fn finance_dev_queries_are_paginated_and_read_only() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        connection.execute_batch("CREATE TABLE finance_test(id INTEGER, label TEXT); INSERT INTO finance_test VALUES(1, 'uno'), (2, 'dos');").expect("fixture");

        let result = finance_dev_query(
            &connection,
            "SELECT id, label FROM finance_test ORDER BY id",
            1,
            1,
        )
        .expect("dev query");

        assert_eq!(result.columns, ["id", "label"]);
        assert_eq!(result.total_rows, 2);
        assert_eq!(
            result.rows,
            vec![vec![Some("2".into()), Some("dos".into())]]
        );
        assert!(validate_finance_dev_sql("DELETE FROM finance_test").is_err());
        assert!(
            validate_finance_dev_sql("SELECT * FROM finance_test; DELETE FROM finance_test")
                .is_err()
        );
    }

    #[test]
    fn finance_dev_seed_populates_every_entity_for_two_months_without_duplicates() {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");

        seed_finance_demo_data(&mut connection).expect("first demo seed");
        seed_finance_demo_data(&mut connection).expect("idempotent demo seed");

        // What was applied and the budget notices sent are bookkeeping, not data.
        for table in FINANCE_DEV_TABLES.iter().filter(|table| !matches!(**table, "finance_applied_requests" | "finance_budget_alerts")) {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("finance table count");
            assert!(count > 0, "{table} should contain demo data");
        }
        let periods: i64 = connection
            .query_row(
                "SELECT COUNT(DISTINCT substr(effective_date, 1, 7)) FROM finance_transactions WHERE id LIKE 'dev-%'",
                [],
                |row| row.get(0),
            )
            .expect("demo transaction periods");
        assert!(periods >= 2);
        let salary_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM finance_salary_receipts WHERE id LIKE 'dev-%'",
                [],
                |row| row.get(0),
            )
            .expect("demo salaries");
        assert_eq!(salary_count, 2);
    }

    #[test]
    fn classifies_ticket_arithmetic_errors_as_validation() {
        let error = FinanceCommandError::from(
            "La compra no puede confirmarse: existe una diferencia de 10.00.".to_string(),
        );
        assert_eq!(error.code, "validation");
    }

    #[test]
    fn classifies_an_already_registered_ticket_as_conflict() {
        let error = FinanceCommandError::from("El ticket ya fue registrado anteriormente.");

        assert_eq!(error.code, "conflict");
    }

    #[test]
    fn clears_finance_data_and_restores_default_categories_without_removing_migrations() {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection
            .execute(
                "INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES(?1,'Efectivo','cash','ARS','100',1,'now','now')",
                params!["account"],
            )
            .expect("account fixture");
        connection
            .execute(
                "INSERT INTO finance_transactions(id,transaction_type,amount,currency,effective_date,account_id,description,source,status,created_at,updated_at) VALUES('transaction','expense','10','ARS','2026-08-31',?1,'Prueba','manual','confirmed','now','now')",
                params!["account"],
            )
            .expect("transaction fixture");

        clear_finance_data(&mut connection).expect("clear finance data");

        for table in [
            "finance_accounts",
            "finance_transactions",
            "finance_source_artifacts",
            "finance_purchases",
            "finance_salary_receipts",
            "finance_savings_reserves",
            "finance_installment_plans",
            "finance_review_items",
            "finance_link_log",
            "finance_loans",
            "finance_loan_installments",
            "finance_budgets",
            "finance_applied_requests",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("finance table count");
            assert_eq!(count, 0, "{table} should be empty");
        }
        let categories: Vec<String> = connection
            .prepare("SELECT name FROM finance_categories ORDER BY name")
            .expect("default category query")
            .query_map([], |row| row.get(0))
            .expect("default category rows")
            .collect::<Result<_, _>>()
            .expect("default categories");
        assert_eq!(categories.len(), 10);
        assert!(categories.iter().any(|name| name == "Alimentación"));
        assert!(categories.iter().any(|name| name == "Otros"));
        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM notia_schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("migration count");
        assert!(migration_count > 0);
    }

    #[test]
    fn calculates_savings_from_confirmed_and_corrected_movements_only() {
        let mut balance = parse_cents("100.00").expect("opening balance");
        apply_savings_movement(&mut balance, "contribution", "50.25", "confirmed");
        apply_savings_movement(&mut balance, "withdrawal", "10.00", "confirmed");
        apply_savings_movement(&mut balance, "loss", "5.25", "pending");
        apply_savings_movement(&mut balance, "return", "1.00", "corrected");
        assert_eq!(format_cents(balance), "141.25");
    }

    #[test]
    fn savings_transfers_do_not_become_income_or_expense() {
        let movement_types = ["contribution", "withdrawal"];
        assert!(movement_types
            .iter()
            .all(|kind| !matches!(*kind, "income" | "expense")));
    }

    #[test]
    fn withdrawals_require_an_auditable_reason() {
        let movement = FinanceSavingsMovement {
            id: "movement".into(),
            reserve_id: "reserve".into(),
            account_id: Some("account".into()),
            movement_type: "withdrawal".into(),
            amount: "10.00".into(),
            currency: "ARS".into(),
            effective_date: "2026-08-29".into(),
            description: "Retiro".into(),
            reason: None,
            source: "manual".into(),
            status: "confirmed".into(),
            actor_user_id: None,
            actor_library_user_id: None,
            linked_transaction_id: None,
        };
        assert!(validate_savings_movement(&movement).is_err());
        assert!(validate_savings_movement(&FinanceSavingsMovement {
            reason: Some("Compra futura".into()),
            ..movement
        })
        .is_ok());
    }

    #[test]
    fn currencies_are_accumulated_independently() {
        let mut totals = std::collections::BTreeMap::new();
        super::add_currency_total(&mut totals, "ARS", "10.25");
        super::add_currency_total(&mut totals, "USD", "2.50");
        assert_eq!(totals.get("ARS"), Some(&1025));
        assert_eq!(totals.get("USD"), Some(&250));
    }

    #[test]
    fn limits_finance_history_to_twelve_months() {
        assert_eq!(
            super::finance_history_start_period("2026-09"),
            Ok("2025-10".into())
        );
        assert_eq!(
            super::finance_history_start_period("2026-01"),
            Ok("2025-02".into())
        );
    }

    #[test]
    fn a_service_occurrence_is_saved_and_replaced_keeping_its_creation() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection
            .execute("INSERT INTO finance_services(id,name,normalized_name,category_id,currency,expected_amount,modality,active,created_at,updated_at) VALUES('movistar','Movistar','movistar','default-expense-services','ARS','84573.99','fixed',1,'now','now')", [])
            .expect("service fixture");
        let save = |paid: &str, timestamp: &str| {
            connection.execute(
                super::UPSERT_SERVICE_OCCURRENCE_SQL,
                params!["service-occurrence:movistar:2026-10", "movistar", "2026-10", "84573.99", paid, "2026-10-05", "accepted", None::<String>, None::<String>, None::<String>, None::<String>, None::<String>, "ai", timestamp],
            )
        };

        save("84573.99", "t1").expect("first save");
        save("84600.00", "t2").expect("replacing save");

        let (paid, created, updated): (String, String, String) = connection
            .query_row("SELECT paid_amount,created_at,updated_at FROM finance_service_occurrences WHERE service_id='movistar' AND period='2026-10'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("occurrence");
        assert_eq!((paid.as_str(), created.as_str(), updated.as_str()), ("84600.00", "t1", "t2"));
    }

    #[test]
    fn cards_and_services_are_compared_with_the_salary_that_paid_them() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection
            .execute_batch(
                "INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES
                    ('bank','Banco','bank','ARS','0',1,'now','now'),
                    ('card','Visa','credit_card','ARS','0',1,'now','now');
                 INSERT INTO finance_salary_receipts(id,period,payment_date,employer,gross_amount,deductions_total,net_amount,currency,account_id,validation_status,created_at,updated_at)
                    VALUES('august','2026-08','2026-08-31','Empresa','1200.00','200.00','1000.00','ARS','bank','confirmed','now','now');
                 INSERT INTO finance_source_artifacts(id,source_type,reference,created_at) VALUES('artifact','credit_card_statement','resumen.pdf','now');
                 INSERT INTO finance_credit_card_statements(id,account_id,issuer,period,closing_date,due_date,currency,total_due,source_artifact_id,validation_status,created_at,updated_at)
                    VALUES('statement','card','Banco','2026-08','2026-08-27','2026-09-04','ARS','250.00','artifact','confirmed','now','now');
                 INSERT INTO finance_services(id,name,normalized_name,category_id,currency,expected_amount,modality,active,created_at,updated_at)
                    VALUES('internet','Internet','internet','default-expense-services','ARS','100.00','fixed',1,'now','now');
                 INSERT INTO finance_service_occurrences(id,service_id,period,expected_amount,paid_amount,effective_date,status,source,created_at,updated_at)
                    VALUES('occurrence','internet','2026-09','100.00','100.00','2026-09-10','accepted','app','now','now');",
            )
            .expect("fixture");

        let history = super::finance_debt_ratio_history(&connection, "2026-09").expect("history");

        // August has salary but nothing paid with it: no point.
        assert_eq!(history.len(), 1);
        let september = &history[0];
        assert_eq!(september.period, "2026-09");
        assert_eq!(september.debt_by_currency.get("ARS").map(String::as_str), Some("250.00"));
        assert_eq!(september.services_by_currency.get("ARS").map(String::as_str), Some("100.00"));
        assert_eq!(september.salary_by_currency.get("ARS").map(String::as_str), Some("1000.00"));
    }

    #[test]
    fn movements_of_documents_name_their_owner() {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        seed_finance_demo_data(&mut connection).expect("demo data");

        let owner = |id: &str| transaction_owner(&connection, id).expect("owner lookup");
        assert_eq!(owner("dev-tx-card-jul"), Some("resumen de tarjeta"));
        assert_eq!(owner("dev-tx-ticket-jul"), Some("ticket"));
        assert_eq!(owner("dev-tx-salary-aug"), Some("recibo de sueldo"));
        assert_eq!(owner("dev-tx-exchange-aug"), Some("movimiento de ahorro"));
        assert_eq!(owner("dev-tx-card-unpaid"), None);
    }
}
