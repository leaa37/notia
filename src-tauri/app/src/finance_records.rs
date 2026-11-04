use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::finance::{
    format_cents, now, parse_cents, sync_context, valid_iso_date, valid_service_period,
    validate_context, FinanceCommandResult, FinanceContext,
};
use crate::finance_matching::{
    self, add_months, find_merchant_for_descriptor, last_merchant_category, CardLine, LineMatch,
    LinkOutcome, ReviewOption, CARD_CHARGES_CATEGORY, CARD_UNPAID,
};
use crate::finance_reconciliation::{
    reconcile_card_service_consumption, CardServiceReconciliation, ReconciliationInput,
    ReconciliationLine, ReconciliationOccurrence, ReconciliationService,
};

const MAX_RECEIPT_ROUNDING_DISCREPANCY_CENTS: u128 = 1;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseItem {
    pub id: String,
    pub original_description: String,
    pub normalized_description: Option<String>,
    pub quantity: String,
    pub unit_price: String,
    pub discount_amount: String,
    pub line_total: String,
    pub category_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseRecord {
    pub id: String,
    pub account_id: String,
    pub category_id: Option<String>,
    pub service_id: Option<String>,
    pub merchant_name: String,
    pub observed_at: String,
    pub currency: String,
    pub subtotal_amount: String,
    pub discount_amount: String,
    pub tax_amount: String,
    pub total_amount: String,
    pub status: String,
    pub source_reference: Option<String>,
    pub raw_extraction: Option<String>,
    pub content_hash: Option<String>,
    pub items: Vec<PurchaseItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseValidation {
    pub valid: bool,
    pub calculated_total: String,
    pub discrepancy: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPurchase {
    pub purchase: PurchaseRecord,
    pub validation: PurchaseValidation,
    /// Links made on save and questions left for the person.
    #[serde(flatten)]
    pub outcome: LinkOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePurchasePayload {
    pub context: FinanceContext,
    pub purchase: PurchaseRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPeriodPayload {
    pub context: FinanceContext,
    pub from: Option<String>,
    pub to: Option<String>,
    pub merchant_id: Option<String>,
    pub product_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseSummary {
    pub id: String,
    pub account_id: Option<String>,
    pub transaction_id: Option<String>,
    pub service_id: Option<String>,
    pub merchant_name: String,
    pub observed_at: String,
    pub currency: String,
    pub total_amount: String,
    pub status: String,
    pub item_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceObservation {
    pub id: String,
    pub product_id: String,
    pub product_name: String,
    pub merchant_name: Option<String>,
    pub observed_at: String,
    pub currency: String,
    pub quantity: String,
    pub unit_price: String,
    pub discount_amount: String,
    pub final_amount: String,
    pub status: String,
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn ensure_money(value: &str, field: &str) -> Result<i128, String> {
    parse_cents(value).map_err(|_| format!("{field} debe ser un importe con hasta dos decimales."))
}

fn purchase_storage_error(step: &str, error: rusqlite::Error) -> String {
    format!("finance_purchase.{step}: {error}")
}

/// Why a ticket does not add up, with the sum that was made, so the agent
/// looks for the missing or extra line instead of padding the discount.
pub(crate) fn purchase_mismatch(record: &PurchaseRecord, validation: &PurchaseValidation) -> String {
    format!(
        "El ticket no cierra: la suma de las líneas da {}, con descuentos de {} e impuestos de {} da {}, pero el total impreso es {} (diferencia de {}). \
         Revisá si falta o sobra una línea (un producto anulado se marca con voided: true y su línea negativa no se manda), los descuentos y si los impuestos ya están incluidos en los precios; \
         no cambies el descuento solo para que cuadre.",
        record.subtotal_amount, record.discount_amount, record.tax_amount, validation.calculated_total, record.total_amount, validation.discrepancy
    )
}

pub(crate) fn validate_purchase(record: &PurchaseRecord) -> Result<PurchaseValidation, String> {
    if record.id.trim().is_empty()
        || record.account_id.trim().is_empty()
        || record.merchant_name.trim().is_empty()
        || !matches!(record.currency.as_str(), "ARS" | "USD")
        || !matches!(
            record.status.as_str(),
            "pending" | "confirmed" | "corrected"
        )
        || !valid_date(&record.observed_at)
        || record.items.is_empty()
    {
        return Err(
            "La compra requiere identificador, cuenta, comercio, fecha, moneda e items.".into(),
        );
    }
    let subtotal = ensure_money(&record.subtotal_amount, "El subtotal")?;
    let discount = ensure_money(&record.discount_amount, "El descuento")?;
    let tax = ensure_money(&record.tax_amount, "Los impuestos")?;
    let total = ensure_money(&record.total_amount, "El total")?;
    let mut lines = 0_i128;
    for item in &record.items {
        if item.original_description.trim().is_empty() {
            return Err("Cada línea requiere una descripción original.".into());
        }
        ensure_money(&item.unit_price, "El precio unitario")?;
        ensure_money(&item.discount_amount, "El descuento de línea")?;
        lines = lines
            .checked_add(ensure_money(&item.line_total, "El total de línea")?)
            .ok_or_else(|| "El total excede el límite admitido.".to_string())?;
        if item
            .quantity
            .parse::<f64>()
            .ok()
            .filter(|value| *value > 0.0)
            .is_none()
        {
            return Err("La cantidad de cada línea debe ser positiva.".into());
        }
    }
    if lines != subtotal {
        return Err(format!(
            "La suma de líneas ({}) no coincide con el subtotal ({}).",
            format_cents(lines),
            format_cents(subtotal)
        ));
    }
    let exclusive_tax_total = subtotal - discount + tax;
    let included_tax_total = subtotal - discount;
    // Argentine consumer receipts commonly show VAT as informational detail
    // already included in each line and in the printed total. Choose the
    // interpretation closest to the printed total before applying the narrow
    // one-cent tolerance used for explicit fiscal rounding adjustments.
    let calculated = if included_tax_total.abs_diff(total) < exclusive_tax_total.abs_diff(total) {
        included_tax_total
    } else {
        exclusive_tax_total
    };
    let discrepancy = total - calculated;
    Ok(PurchaseValidation {
        valid: discrepancy.unsigned_abs() <= MAX_RECEIPT_ROUNDING_DISCREPANCY_CENTS,
        calculated_total: format_cents(calculated),
        discrepancy: format_cents(discrepancy),
    })
}

fn valid_date(value: &str) -> bool {
    value.get(..10).is_some_and(valid_iso_date)
}

fn hash_purchase(record: &PurchaseRecord) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize(&record.merchant_name));
    hasher.update(record.observed_at.as_bytes());
    hasher.update(record.currency.as_bytes());
    hasher.update(record.total_amount.as_bytes());
    for item in &record.items {
        hasher.update(normalize(&item.original_description));
        hasher.update(item.quantity.as_bytes());
        hasher.update(item.unit_price.as_bytes());
        hasher.update(item.discount_amount.as_bytes());
        hasher.update(item.line_total.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn finance_save_purchase(
    app: crate::host::AppHandle,
    payload: SavePurchasePayload,
) -> FinanceCommandResult<SavedPurchase> {
    let mut connection = validate_context(&payload.context, &app)?;
    let saved = store_purchase(&mut connection, &payload.purchase, &payload.context)?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(saved)
}

pub(crate) fn store_purchase(
    connection: &mut rusqlite::Connection,
    record: &PurchaseRecord,
    context: &FinanceContext,
) -> FinanceCommandResult<SavedPurchase> {
    let validation = validate_purchase(record)?;
    if record.status != "pending" && !validation.valid {
        return Err(purchase_mismatch(record, &validation).into());
    }
    let (account_type, account_currency): (String, String) = connection
        .query_row(
            "SELECT account_type,currency FROM finance_accounts WHERE id=?1 AND active=1",
            [&record.account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "La cuenta de la compra no existe o está inactiva.".to_string())?;
    // Credit cards take pesos and dollars; other accounts have one currency.
    let credit_card = account_type == "credit_card";
    if !credit_card && account_currency != record.currency {
        return Err("La moneda de la compra no coincide con la cuenta.".into());
    }
    if let Some(category_id) = record.category_id.as_deref() {
        let category_kind: String = connection
            .query_row(
                "SELECT kind FROM finance_categories WHERE id=?1 AND active=1",
                [category_id],
                |row| row.get(0),
            )
            .map_err(|_| "La categoría de la compra no existe o está inactiva.".to_string())?;
        if category_kind != "expense" {
            return Err("La compra requiere una categoría de gasto.".into());
        }
    }
    if let Some(service_id) = record.service_id.as_deref() {
        let service_currency: String = connection
            .query_row(
                "SELECT currency FROM finance_services WHERE id=?1 AND active=1",
                [service_id],
                |row| row.get(0),
            )
            .map_err(|_| "El servicio de la compra no existe o está inactivo.".to_string())?;
        if service_currency != record.currency {
            return Err("La moneda de la compra no coincide con el servicio.".into());
        }
    }
    let fingerprint = record
        .content_hash
        .clone()
        .unwrap_or_else(|| hash_purchase(record));
    let duplicate: Option<String> = connection
        .query_row(
            "SELECT id FROM finance_source_artifacts WHERE content_hash = ?1",
            [&fingerprint],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(artifact_id) = duplicate {
        let belongs_to_record: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM finance_purchases WHERE id = ?1 AND source_artifact_id = ?2",
                params![record.id, artifact_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if belongs_to_record == 0 {
            return Err("El ticket ya fue registrado anteriormente.".into());
        }
    }
    let linked_transaction_id: Option<String> = connection
        .query_row(
            "SELECT transaction_id FROM finance_purchases WHERE id = ?1",
            [&record.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let timestamp = now();
    let artifact_id = format!("ticket:{}", record.id);
    let receipt_id = format!("receipt:{}", record.id);
    let own_transaction_id = format!("purchase:{}", record.id);
    let operation_fingerprint = format!("ticket:{fingerprint}");
    let purchase_date = &record.observed_at[..10];
    let description = format!("Compra en {}", record.merchant_name.trim());
    let transaction = connection
        .transaction()
        .map_err(|error| purchase_storage_error("begin", error))?;
    let mut outcome = LinkOutcome::default();
    let merchant_id = finance_matching::resolve_merchant(&transaction, &record.merchant_name, &mut outcome)?;
    transaction.execute(
        "INSERT INTO finance_source_artifacts (id,source_type,reference,raw_text,content_hash,created_at) VALUES (?1,'ticket',?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET reference=excluded.reference,raw_text=excluded.raw_text,content_hash=excluded.content_hash",
        params![artifact_id, record.source_reference, record.raw_extraction, fingerprint, timestamp],
    ).map_err(|error| purchase_storage_error("artifact", error))?;
    transaction.execute(
        "INSERT INTO finance_receipts (id,source_artifact_id,receipt_type,validation_status,created_at,updated_at) VALUES (?1,?2,'ticket',?3,?4,?4) ON CONFLICT(id) DO UPDATE SET validation_status=excluded.validation_status,updated_at=excluded.updated_at",
        params![receipt_id, artifact_id, record.status, timestamp],
    ).map_err(|error| purchase_storage_error("receipt", error))?;
    let mut waits_for_statement = false;
    match linked_transaction_id.as_deref().filter(|id| *id != own_transaction_id) {
        // A statement line already paid this ticket: the ticket only names it.
        Some(line_transaction_id) => {
            transaction.execute(
                "UPDATE finance_transactions SET merchant_id=?1,category_id=COALESCE(?2,category_id),
                        service_id=COALESCE(?3,service_id),description=?4,updated_at=?5
                 WHERE id=?6",
                params![merchant_id, record.category_id, record.service_id, description, timestamp, line_transaction_id],
            ).map_err(|error| purchase_storage_error("transaction", error))?;
        }
        None => {
            let paid_by_line: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM finance_credit_card_statement_items WHERE transaction_id=?1)",
                    [&own_transaction_id],
                    |row| row.get(0),
                )
                .map_err(|error| purchase_storage_error("line_lookup", error))?;
            waits_for_statement = credit_card && !paid_by_line && record.status != "pending";
            let status = if paid_by_line {
                "confirmed"
            } else if waits_for_statement {
                CARD_UNPAID
            } else {
                record.status.as_str()
            };
            // A card purchase counts when its statement is paid; until then,
            // and for a line that already paid it, the purchase day is kept.
            let effective_date = if paid_by_line {
                transaction
                    .query_row(
                        "SELECT effective_date FROM finance_transactions WHERE id=?1",
                        [&own_transaction_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| purchase_storage_error("line_date", error))?
            } else {
                purchase_date.to_string()
            };
            let card_purchase_date = credit_card.then_some(purchase_date);
            transaction.execute(
                "INSERT INTO finance_transactions (id,transaction_type,amount,currency,effective_date,purchase_date,account_id,category_id,description,source,status,source_artifact_id,service_id,merchant_id,operation_fingerprint,source_reference,raw_source,actor_library_user_id,created_at,updated_at) VALUES (?1,'expense',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18) ON CONFLICT(id) DO UPDATE SET amount=excluded.amount,effective_date=excluded.effective_date,purchase_date=excluded.purchase_date,account_id=excluded.account_id,category_id=excluded.category_id,description=excluded.description,status=excluded.status,service_id=excluded.service_id,merchant_id=excluded.merchant_id,source_reference=excluded.source_reference,raw_source=excluded.raw_source,actor_library_user_id=excluded.actor_library_user_id,deleted_at=NULL,updated_at=excluded.updated_at",
                params![own_transaction_id, record.total_amount, record.currency, effective_date, card_purchase_date, record.account_id, record.category_id, description, context.source, status, artifact_id, record.service_id, merchant_id, operation_fingerprint, record.source_reference, record.raw_extraction, context.actor_library_user_id, timestamp],
            ).map_err(|error| purchase_storage_error("transaction", error))?;
        }
    }
    transaction.execute(
        "INSERT INTO finance_purchases (id,transaction_id,merchant_id,service_id,observed_at,currency,total_amount,source_artifact_id,subtotal_amount,discount_amount,tax_amount,validation_status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13) ON CONFLICT(id) DO UPDATE SET merchant_id=excluded.merchant_id,service_id=excluded.service_id,observed_at=excluded.observed_at,total_amount=excluded.total_amount,subtotal_amount=excluded.subtotal_amount,discount_amount=excluded.discount_amount,tax_amount=excluded.tax_amount,validation_status=excluded.validation_status,updated_at=excluded.updated_at",
         params![record.id, own_transaction_id, merchant_id, record.service_id, record.observed_at, record.currency, record.total_amount, artifact_id, record.subtotal_amount, record.discount_amount, record.tax_amount, record.status, timestamp],
    ).map_err(|error| purchase_storage_error("purchase", error))?;
    transaction.execute("DELETE FROM finance_price_observations WHERE purchase_item_id IN (SELECT id FROM finance_purchase_items WHERE purchase_id = ?1)", [&record.id]).map_err(|error| purchase_storage_error("clear_prices", error))?;
    transaction
        .execute(
            "DELETE FROM finance_purchase_items WHERE purchase_id = ?1",
            [&record.id],
        )
        .map_err(|error| purchase_storage_error("clear_items", error))?;
    for item in &record.items {
        let product_name = item
            .normalized_description
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&item.original_description);
        let product_id = finance_matching::resolve_product(&transaction, product_name, &mut outcome)?;
        transaction.execute(
            "INSERT INTO finance_purchase_items (id,purchase_id,product_id,original_description,normalized_description,quantity,unit_price,discount_amount,line_total,currency,category_id,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![item.id, record.id, product_id, item.original_description.trim(), item.normalized_description, item.quantity, item.unit_price, item.discount_amount, item.line_total, record.currency, item.category_id, timestamp],
        ).map_err(|error| purchase_storage_error("item", error))?;
        transaction.execute(
            "INSERT INTO finance_price_observations (id,purchase_item_id,product_id,merchant_id,observed_at,currency,quantity,unit_price,discount_amount,final_amount,status,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![format!("price:{}", item.id), item.id, product_id, merchant_id, record.observed_at, record.currency, item.quantity, item.unit_price, item.discount_amount, item.line_total, record.status, timestamp],
        ).map_err(|error| purchase_storage_error("price", error))?;
    }
    if waits_for_statement {
        finance_matching::link_unpaid_expense(&transaction, &own_transaction_id, &mut outcome)?;
    }
    transaction
        .commit()
        .map_err(|error| purchase_storage_error("commit", error))?;
    let mut saved = record.clone();
    saved.content_hash = Some(fingerprint);
    Ok(SavedPurchase {
        purchase: saved,
        validation,
        outcome,
    })
}

pub fn finance_list_purchases(
    app: crate::host::AppHandle,
    payload: ListPeriodPayload,
) -> FinanceCommandResult<Vec<PurchaseSummary>> {
    let connection = validate_context(&payload.context, &app)?;
    let mut statement = connection.prepare(
        "SELECT p.id,t.account_id,p.transaction_id,p.service_id,m.name,p.observed_at,p.currency,p.total_amount,p.validation_status,COUNT(i.id) FROM finance_purchases p LEFT JOIN finance_transactions t ON t.id=p.transaction_id LEFT JOIN finance_merchants m ON m.id=p.merchant_id LEFT JOIN finance_purchase_items i ON i.purchase_id=p.id WHERE (?1 IS NULL OR p.observed_at >= ?1) AND (?2 IS NULL OR p.observed_at <= ?2) AND (?3 IS NULL OR p.merchant_id=?3) AND (?4 IS NULL OR EXISTS (SELECT 1 FROM finance_purchase_items pi WHERE pi.purchase_id=p.id AND pi.product_id=?4)) GROUP BY p.id ORDER BY p.observed_at DESC LIMIT 500"
    ).map_err(|error| error.to_string())?;
    let purchases = statement
        .query_map(
            params![
                payload.from,
                payload.to,
                payload.merchant_id,
                payload.product_id
            ],
            |row| {
                Ok(PurchaseSummary {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    transaction_id: row.get(2)?,
                    service_id: row.get(3)?,
                    merchant_name: row.get(4)?,
                    observed_at: row.get(5)?,
                    currency: row.get(6)?,
                    total_amount: row.get(7)?,
                    status: row.get(8)?,
                    item_count: row.get(9)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(purchases)
}

/// The lines of a purchase (a ticket), for reading it by id.
pub(crate) fn purchase_items(app: &crate::host::AppHandle, context: &FinanceContext, purchase_id: &str) -> FinanceCommandResult<Vec<serde_json::Value>> {
    let connection = validate_context(context, app)?;
    let mut statement = connection
        .prepare(
            "SELECT i.id,i.original_description,i.normalized_description,i.quantity,i.unit_price,i.discount_amount,i.line_total,i.currency,c.name,p.name FROM finance_purchase_items i LEFT JOIN finance_categories c ON c.id=i.category_id LEFT JOIN finance_products p ON p.id=i.product_id WHERE i.purchase_id=?1 ORDER BY i.rowid",
        )
        .map_err(|error| error.to_string())?;
    let items = statement
        .query_map(params![purchase_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "description": row.get::<_, String>(1)?,
                "normalizedDescription": row.get::<_, Option<String>>(2)?,
                "quantity": row.get::<_, Option<String>>(3)?,
                "unitPrice": row.get::<_, Option<String>>(4)?,
                "discountAmount": row.get::<_, Option<String>>(5)?,
                "lineTotal": row.get::<_, String>(6)?,
                "currency": row.get::<_, String>(7)?,
                "category": row.get::<_, Option<String>>(8)?,
                "product": row.get::<_, Option<String>>(9)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(items)
}

pub fn finance_list_price_history(
    app: crate::host::AppHandle,
    payload: ListPeriodPayload,
) -> FinanceCommandResult<Vec<PriceObservation>> {
    let connection = validate_context(&payload.context, &app)?;
    let mut statement = connection.prepare(
        "SELECT o.id,o.product_id,p.name,m.name,o.observed_at,o.currency,o.quantity,o.unit_price,o.discount_amount,o.final_amount,o.status FROM finance_price_observations o JOIN finance_products p ON p.id=o.product_id LEFT JOIN finance_merchants m ON m.id=o.merchant_id WHERE o.status IN ('confirmed','corrected') AND (?1 IS NULL OR o.observed_at >= ?1) AND (?2 IS NULL OR o.observed_at <= ?2) AND (?3 IS NULL OR o.merchant_id=?3) AND (?4 IS NULL OR o.product_id=?4) ORDER BY o.observed_at DESC LIMIT 1000"
    ).map_err(|error| error.to_string())?;
    let prices = statement
        .query_map(
            params![
                payload.from,
                payload.to,
                payload.merchant_id,
                payload.product_id
            ],
            |row| {
                Ok(PriceObservation {
                    id: row.get(0)?,
                    product_id: row.get(1)?,
                    product_name: row.get(2)?,
                    merchant_name: row.get(3)?,
                    observed_at: row.get(4)?,
                    currency: row.get(5)?,
                    quantity: row.get(6)?,
                    unit_price: row.get(7)?,
                    discount_amount: row.get(8)?,
                    final_amount: row.get(9)?,
                    status: row.get(10)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(prices)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SalaryConcept {
    pub id: String,
    pub name: String,
    pub concept_type: String,
    pub amount: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SalaryReceipt {
    pub id: String,
    pub period: String,
    pub payment_date: String,
    pub employer: String,
    pub gross_amount: String,
    pub deductions_total: String,
    pub net_amount: String,
    pub currency: String,
    pub account_id: String,
    pub status: String,
    #[serde(default)]
    pub signed_document: bool,
    #[serde(default, skip_deserializing)]
    pub created_at: Option<String>,
    pub source_reference: Option<String>,
    pub raw_extraction: Option<String>,
    pub concepts: Vec<SalaryConcept>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSalaryPayload {
    pub context: FinanceContext,
    pub salary: SalaryReceipt,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalaryEvolution {
    pub salary: SalaryReceipt,
    pub gross_change: String,
    pub net_change: String,
    pub deductions_change: String,
    pub net_change_percent: Option<String>,
}

fn validate_salary_receipt(salary: &SalaryReceipt) -> Result<(), String> {
    if salary.id.trim().is_empty()
        || salary.employer.trim().is_empty()
        || salary.account_id.trim().is_empty()
        || !valid_service_period(&salary.period)
        || !valid_date(&salary.payment_date)
        || !matches!(salary.currency.as_str(), "ARS" | "USD")
        || !matches!(
            salary.status.as_str(),
            "pending" | "confirmed" | "corrected"
        )
    {
        return Err(
            "El recibo requiere período, empleador, cuenta, fecha, moneda y estado válidos.".into(),
        );
    }
    let gross = ensure_money(&salary.gross_amount, "El bruto")?;
    let deductions = ensure_money(&salary.deductions_total, "Los descuentos")?;
    let net = ensure_money(&salary.net_amount, "El neto")?;
    let has_pdf_evidence = salary
        .source_reference
        .as_deref()
        .is_some_and(|reference| reference.to_ascii_lowercase().ends_with(".pdf"));
    if salary.employer.chars().count() > 160
        || salary.concepts.len() > 200
        || salary
            .source_reference
            .as_deref()
            .map(|value| value.len() > 512)
            .unwrap_or(false)
        || salary
            .raw_extraction
            .as_deref()
            .map(|value| value.len() > 20_000)
            .unwrap_or(false)
    {
        return Err("El recibo supera los límites de texto o conceptos admitidos.".into());
    }
    if salary.signed_document && !has_pdf_evidence {
        return Err("Un recibo marcado como firmado requiere una evidencia PDF.".into());
    }
    if salary.status != "pending"
        && gross - deductions != net
        && !(salary.signed_document && has_pdf_evidence)
    {
        return Err(format!(
            "El neto debería ser {} según bruto menos descuentos.",
            format_cents(gross - deductions)
        ));
    }
    for concept in &salary.concepts {
        ensure_money(&concept.amount, "El importe del concepto")?;
        if concept.id.trim().is_empty()
            || concept.name.trim().is_empty()
            || !matches!(concept.concept_type.as_str(), "earning" | "deduction")
        {
            return Err(
                "Cada concepto requiere identificador, nombre y tipo earning o deduction.".into(),
            );
        }
    }
    Ok(())
}

pub fn finance_save_salary(
    app: crate::host::AppHandle,
    payload: SaveSalaryPayload,
) -> FinanceCommandResult<SalaryReceipt> {
    let salary = &payload.salary;
    validate_salary_receipt(salary)?;
    let mut connection = validate_context(&payload.context, &app)?;
    let account_currency: Option<String> = connection
        .query_row(
            "SELECT currency FROM finance_accounts WHERE id = ?1 AND active = 1",
            [&salary.account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(account_currency) = account_currency else {
        return Err("La cuenta del recibo no existe o está inactiva.".into());
    };
    if account_currency != salary.currency {
        return Err("La moneda del recibo debe coincidir con la moneda de la cuenta.".into());
    }
    let duplicate_salary = {
        let mut statement = connection
            .prepare(
                "SELECT id,employer FROM finance_salary_receipts
                 WHERE period=?1 AND id<>?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![salary.period, salary.id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let expected_employer = normalize(&salary.employer);
        let mut duplicate = false;
        for row in rows {
            let (_, employer) = row.map_err(|error| error.to_string())?;
            if normalize(&employer) == expected_employer {
                duplicate = true;
                break;
            }
        }
        duplicate
    };
    if duplicate_salary {
        return Err("El recibo de sueldo ya fue registrado anteriormente.".into());
    }
    let timestamp = now();
    let artifact_id = format!("salary:{}", salary.id);
    let transaction_id = format!("salary-income:{}", salary.id);
    let receipt_id = format!("salary-receipt:{}", salary.id);
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO finance_source_artifacts (id,source_type,reference,raw_text,content_hash,created_at) VALUES (?1,'salary',?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET reference=excluded.reference,raw_text=excluded.raw_text",params![artifact_id,salary.source_reference,salary.raw_extraction,format!("salary:{}:{}",normalize(&salary.employer),salary.period),timestamp]).map_err(|error|error.to_string())?;
    transaction.execute("INSERT INTO finance_receipts (id,source_artifact_id,receipt_type,validation_status,created_at,updated_at) VALUES (?1,?2,'salary',?3,?4,?4) ON CONFLICT(id) DO UPDATE SET validation_status=excluded.validation_status,updated_at=excluded.updated_at",params![receipt_id,artifact_id,salary.status,timestamp]).map_err(|error|error.to_string())?;
    transaction.execute("INSERT INTO finance_transactions (id,transaction_type,amount,currency,effective_date,account_id,description,source,status,source_artifact_id,operation_fingerprint,source_reference,raw_source,actor_library_user_id,created_at,updated_at) VALUES (?1,'income',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14) ON CONFLICT(id) DO UPDATE SET amount=excluded.amount,effective_date=excluded.effective_date,account_id=excluded.account_id,source=excluded.source,status=excluded.status,source_reference=excluded.source_reference,raw_source=excluded.raw_source,actor_library_user_id=excluded.actor_library_user_id,updated_at=excluded.updated_at",params![transaction_id,salary.net_amount,salary.currency,&salary.payment_date[..10],salary.account_id,format!("Sueldo {} · {}",salary.employer,salary.period),payload.context.source,salary.status,artifact_id,format!("salary:{}:{}",normalize(&salary.employer),salary.period),salary.source_reference,salary.raw_extraction,payload.context.actor_library_user_id,timestamp]).map_err(|error|error.to_string())?;
    transaction.execute("INSERT INTO finance_salary_receipts (id,period,payment_date,employer,gross_amount,deductions_total,net_amount,currency,account_id,transaction_id,source_artifact_id,validation_status,signed_document,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14) ON CONFLICT(id) DO UPDATE SET payment_date=excluded.payment_date,gross_amount=excluded.gross_amount,deductions_total=excluded.deductions_total,net_amount=excluded.net_amount,account_id=excluded.account_id,validation_status=excluded.validation_status,signed_document=excluded.signed_document,updated_at=excluded.updated_at",params![salary.id,salary.period,salary.payment_date,salary.employer,salary.gross_amount,salary.deductions_total,salary.net_amount,salary.currency,salary.account_id,transaction_id,artifact_id,salary.status,salary.signed_document as i32,timestamp]).map_err(|error|error.to_string())?;
    transaction
        .execute(
            "DELETE FROM finance_salary_concepts WHERE salary_receipt_id=?1",
            [&salary.id],
        )
        .map_err(|error| error.to_string())?;
    for concept in &salary.concepts {
        transaction.execute("INSERT INTO finance_salary_concepts (id,salary_receipt_id,name,concept_type,amount,currency,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![concept.id,salary.id,concept.name,concept.concept_type,concept.amount,salary.currency,timestamp]).map_err(|error|error.to_string())?;
    }
    let persisted_created_at: Option<String> = transaction
        .query_row(
            "SELECT created_at FROM finance_salary_receipts WHERE id=?1",
            [&salary.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    let mut saved = salary.clone();
    saved.created_at = persisted_created_at;
    Ok(saved)
}

pub fn finance_list_salaries(
    app: crate::host::AppHandle,
    payload: ListPeriodPayload,
) -> FinanceCommandResult<Vec<SalaryEvolution>> {
    let connection = validate_context(&payload.context, &app)?;
    let mut salaries = {
        let mut statement = connection
            .prepare(
                "SELECT s.id,s.period,s.payment_date,s.employer,s.gross_amount,
                        s.deductions_total,s.net_amount,s.currency,s.account_id,
                         s.validation_status,s.signed_document,s.created_at,a.reference,a.raw_text
                 FROM finance_salary_receipts s
                 LEFT JOIN finance_source_artifacts a ON a.id=s.source_artifact_id
                 WHERE (?1 IS NULL OR s.period>=?1) AND (?2 IS NULL OR s.period<=?2)
                 ORDER BY s.period",
            )
            .map_err(|error| error.to_string())?;
        let salaries = statement
            .query_map(params![payload.from, payload.to], |row| {
                Ok(SalaryReceipt {
                    id: row.get(0)?,
                    period: row.get(1)?,
                    payment_date: row.get(2)?,
                    employer: row.get(3)?,
                    gross_amount: row.get(4)?,
                    deductions_total: row.get(5)?,
                    net_amount: row.get(6)?,
                    currency: row.get(7)?,
                    account_id: row.get(8)?,
                    status: row.get(9)?,
                    signed_document: row.get::<_, i32>(10)? != 0,
                    created_at: row.get(11)?,
                    source_reference: row.get(12)?,
                    raw_extraction: row.get(13)?,
                    concepts: Vec::new(),
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        salaries
    };
    for salary in &mut salaries {
        let mut concept_statement = connection
            .prepare(
                "SELECT id,name,concept_type,amount
                 FROM finance_salary_concepts
                 WHERE salary_receipt_id=?1 ORDER BY created_at,id",
            )
            .map_err(|error| error.to_string())?;
        salary.concepts = concept_statement
            .query_map([&salary.id], |row| {
                Ok(SalaryConcept {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    concept_type: row.get(2)?,
                    amount: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
    }
    Ok(build_salary_evolution(salaries)?)
}

fn build_salary_evolution(salaries: Vec<SalaryReceipt>) -> Result<Vec<SalaryEvolution>, String> {
    let mut previous: BTreeMap<String, (i128, i128, i128)> = BTreeMap::new();
    let mut result = Vec::new();
    for salary in salaries {
        let gross = ensure_money(&salary.gross_amount, "bruto")?;
        let net = ensure_money(&salary.net_amount, "neto")?;
        let deductions = ensure_money(&salary.deductions_total, "descuentos")?;
        let prior = previous
            .get(&salary.currency)
            .copied()
            .unwrap_or((gross, net, deductions));
        let percent = if prior.1 == 0 {
            None
        } else {
            Some(format!(
                "{:.2}",
                ((net - prior.1) as f64 / prior.1 as f64) * 100.0
            ))
        };
        result.push(SalaryEvolution {
            salary: salary.clone(),
            gross_change: format_cents(gross - prior.0),
            net_change: format_cents(net - prior.1),
            deductions_change: format_cents(deductions - prior.2),
            net_change_percent: percent,
        });
        previous.insert(salary.currency.clone(), (gross, net, deductions));
    }
    result.reverse();
    Ok(result)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreditCardStatementItem {
    pub id: String,
    pub purchase_date: String,
    pub description: String,
    pub amount: String,
    pub currency: String,
    pub item_type: String,
    pub installment_number: Option<u16>,
    pub installment_count: Option<u16>,
    pub transaction_id: Option<String>,
    /// Category for a purchase line without a ticket; the merchant's usual
    /// category is used when absent.
    #[serde(default)]
    pub category_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreditCardStatement {
    pub id: String,
    pub account_id: String,
    pub issuer: String,
    pub card_last_four: Option<String>,
    pub period: String,
    pub closing_date: String,
    pub due_date: String,
    pub currency: String,
    pub previous_balance: String,
    pub payments_amount: String,
    pub credits_amount: String,
    pub purchases_amount: String,
    pub fees_amount: String,
    pub interest_amount: String,
    pub taxes_amount: String,
    pub total_due: String,
    pub minimum_payment: Option<String>,
    pub status: String,
    #[serde(default, skip_deserializing)]
    pub created_at: Option<String>,
    pub source_reference: Option<String>,
    pub raw_extraction: Option<String>,
    pub items: Vec<CreditCardStatementItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCreditCardStatementPayload {
    pub context: FinanceContext,
    pub statement: CreditCardStatement,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCreditCardStatement {
    pub statement: CreditCardStatement,
    pub matched_existing_transactions: usize,
    pub created_transactions: usize,
    pub reconciliation: CardServiceReconciliation,
    pub occurrences: Vec<crate::finance::FinanceServiceOccurrence>,
    /// Links made on save and questions left for the person.
    #[serde(flatten)]
    pub outcome: LinkOutcome,
}

fn validate_credit_card_statement(statement: &CreditCardStatement) -> Result<(), String> {
    if statement.id.trim().is_empty()
        || statement.account_id.trim().is_empty()
        || statement.issuer.trim().is_empty()
        || statement.issuer.len() > 200
        || statement.period.len() != 7
        || !valid_date(&format!("{}-01", statement.period))
        || !valid_date(&statement.closing_date)
        || !valid_date(&statement.due_date)
        || !matches!(statement.currency.as_str(), "ARS" | "USD")
        || !matches!(
            statement.status.as_str(),
            "pending" | "confirmed" | "corrected"
        )
        || statement.items.is_empty()
        || statement.items.len() > 300
        || statement.card_last_four.as_deref().is_some_and(|digits| {
            digits.len() != 4 || !digits.chars().all(|character| character.is_ascii_digit())
        })
        || statement
            .raw_extraction
            .as_deref()
            .is_some_and(|raw| raw.len() > 20_000)
    {
        return Err("El resumen requiere tarjeta, emisor, período, cierre, vencimiento, moneda e items válidos.".into());
    }
    let previous = ensure_money(&statement.previous_balance, "El saldo anterior")?;
    let payments = ensure_money(&statement.payments_amount, "Los pagos")?;
    let credits = ensure_money(&statement.credits_amount, "Los créditos")?;
    let purchases = ensure_money(&statement.purchases_amount, "Las compras")?;
    let fees = ensure_money(&statement.fees_amount, "Los cargos")?;
    let interest = ensure_money(&statement.interest_amount, "Los intereses")?;
    let taxes = ensure_money(&statement.taxes_amount, "Los impuestos")?;
    let total = ensure_money(&statement.total_due, "El total a pagar")?;
    if let Some(minimum) = &statement.minimum_payment {
        ensure_money(minimum, "El pago mínimo")?;
    }
    let mut item_totals: BTreeMap<&str, i128> = BTreeMap::new();
    for item in &statement.items {
        if item.id.trim().is_empty()
            || item.description.trim().is_empty()
            || item.description.len() > 500
            || !valid_date(&item.purchase_date)
            || item.currency != statement.currency
            || !matches!(
                item.item_type.as_str(),
                "purchase" | "fee" | "interest" | "tax" | "payment" | "credit"
            )
        {
            return Err(
                "Cada línea del resumen requiere descripción, fecha, moneda y tipo válidos.".into(),
            );
        }
        let amount = ensure_money(&item.amount, "El importe de la línea")?;
        if amount < 0 {
            return Err("Los importes del resumen deben expresarse como valores positivos.".into());
        }
        *item_totals.entry(item.item_type.as_str()).or_default() += amount;
        if item.installment_number.is_some() != item.installment_count.is_some()
            || item
                .installment_number
                .zip(item.installment_count)
                .is_some_and(|(number, count)| number == 0 || count == 0 || number > count)
        {
            return Err("La cuota del resumen debe indicar un número y total válidos.".into());
        }
    }
    let expected_by_type = [
        ("purchase", purchases),
        ("fee", fees),
        ("interest", interest),
        ("tax", taxes),
        ("payment", payments),
        ("credit", credits),
    ];
    for (kind, expected) in expected_by_type {
        let actual = item_totals.get(kind).copied().unwrap_or_default();
        if actual.abs_diff(expected) > MAX_RECEIPT_ROUNDING_DISCREPANCY_CENTS {
            return Err(format!(
                "La suma de líneas de tipo {kind} ({}) no coincide con el total informado ({}).",
                format_cents(actual),
                format_cents(expected)
            ));
        }
    }
    let calculated = previous - payments - credits + purchases + fees + interest + taxes;
    if calculated.abs_diff(total) > MAX_RECEIPT_ROUNDING_DISCREPANCY_CENTS {
        return Err(format!(
            "El total a pagar no coincide: debería ser {} y se informó {}.",
            format_cents(calculated),
            format_cents(total)
        ));
    }
    Ok(())
}

fn load_card_reconciliation_input(
    connection: &rusqlite::Connection,
    statement: &CreditCardStatement,
    items: &[CreditCardStatementItem],
) -> Result<ReconciliationInput, String> {
    let services = connection
        .prepare("SELECT id,name,provider,currency FROM finance_services ORDER BY id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(ReconciliationService {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                currency: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let occurrences = connection
        .prepare("SELECT service_id,period,paid_amount,transaction_id FROM finance_service_occurrences")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(ReconciliationOccurrence {
                service_id: row.get(0)?,
                period: row.get(1)?,
                paid_amount: row.get(2)?,
                transaction_id: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let statement_counts = matches!(statement.status.as_str(), "confirmed" | "corrected");
    let lines = items
        .iter()
        .map(|item| {
            // Only a confirmed expense with the line's amount can pay a service.
            let confirmed = statement_counts
                && item.transaction_id.as_deref().is_some_and(|transaction_id| {
                    connection
                        .query_row(
                            "SELECT amount,currency FROM finance_transactions
                             WHERE id=?1 AND deleted_at IS NULL AND transaction_type='expense'
                               AND status IN ('confirmed','corrected')",
                            [transaction_id],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                        )
                        .ok()
                        .is_some_and(|(amount, currency)| {
                            currency == item.currency && parse_cents(&amount) == parse_cents(&item.amount)
                        })
                });
            ReconciliationLine {
                id: item.id.clone(),
                transaction_id: item.transaction_id.clone(),
                purchase_date: item.purchase_date.clone(),
                description: item.description.clone(),
                amount: item.amount.clone(),
                currency: item.currency.clone(),
                item_type: item.item_type.clone(),
                confirmed,
            }
        })
        .collect();
    Ok(ReconciliationInput {
        statement_id: statement.id.clone(),
        statement_period: statement.period.clone(),
        lines,
        services,
        occurrences,
    })
}

fn occurrence_from_connection(
    connection: &rusqlite::Connection,
    service_id: &str,
    period: &str,
) -> Result<crate::finance::FinanceServiceOccurrence, String> {
    connection
        .query_row(
            "SELECT id,service_id,period,expected_amount,paid_amount,effective_date,status,
                    transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,
                    source,created_at,updated_at
             FROM finance_service_occurrences WHERE service_id=?1 AND period=?2",
            params![service_id, period],
            |row| {
                Ok(crate::finance::FinanceServiceOccurrence {
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
            },
        )
        .map_err(|error| error.to_string())
}

/// Undoes what a statement line did with an expense once the line no
/// longer uses it. The line's own expense is removed; an expense the line
/// had taken over (a ticket or a loose card expense) waits again for its
/// statement; tickets merged into the line get their own expense back.
fn retire_stale_card_statement_transaction(
    transaction: &rusqlite::Connection,
    item_id: &str,
    transaction_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    let still_used: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM finance_credit_card_statement_items WHERE transaction_id=?1)",
            [transaction_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if still_used {
        return Ok(());
    }
    let linked_occurrence: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM finance_service_occurrences WHERE transaction_id=?1",
            [transaction_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if linked_occurrence > 0 {
        return Err(
            "No se puede quitar una línea ya conciliada con un servicio; desvinculá primero la ocurrencia del servicio."
                .into(),
        );
    }
    let source: Option<String> = transaction
        .query_row(
            "SELECT source FROM finance_transactions WHERE id=?1 AND deleted_at IS NULL",
            [transaction_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    match source.as_deref() {
        Some("credit_card_statement") => {
            let merged_purchases = transaction
                .prepare("SELECT id FROM finance_purchases WHERE transaction_id=?1")
                .map_err(|error| error.to_string())?
                .query_map([transaction_id], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            for purchase_id in merged_purchases {
                let own = format!("purchase:{purchase_id}");
                transaction
                    .execute(
                        "UPDATE finance_transactions SET deleted_at=NULL,status=?1,
                                effective_date=COALESCE(purchase_date,effective_date),updated_at=?2
                         WHERE id=?3",
                        params![CARD_UNPAID, timestamp, own],
                    )
                    .map_err(|error| error.to_string())?;
                transaction
                    .execute(
                        "UPDATE finance_purchases SET transaction_id=?1,updated_at=?2 WHERE id=?3",
                        params![own, timestamp, purchase_id],
                    )
                    .map_err(|error| error.to_string())?;
            }
            transaction
                .execute(
                    "UPDATE finance_transactions SET deleted_at=?1,updated_at=?1 WHERE id=?2",
                    params![timestamp, transaction_id],
                )
                .map_err(|error| error.to_string())?;
        }
        Some(_) => {
            transaction
                .execute(
                    "UPDATE finance_transactions SET status=?1,effective_date=COALESCE(purchase_date,effective_date),
                            updated_at=?2
                     WHERE id=?3",
                    params![CARD_UNPAID, timestamp, transaction_id],
                )
                .map_err(|error| error.to_string())?;
        }
        None => {}
    }
    transaction
        .execute(
            "UPDATE finance_installments SET transaction_id=NULL,status='pending',updated_at=?1 WHERE transaction_id=?2",
            params![timestamp, transaction_id],
        )
        .map_err(|error| error.to_string())?;
    finance_matching::forget_links_for_line(transaction, item_id)
}

/// Removes a statement: its lines, the expenses only the statement had and
/// the service payments it recorded. Tickets and loose card expenses it had
/// taken over wait again for a statement.
pub(crate) fn delete_card_statement(
    connection: &rusqlite::Connection,
    statement_id: &str,
) -> Result<String, String> {
    let artifact_id: String = connection
        .query_row(
            "SELECT source_artifact_id FROM finance_credit_card_statements WHERE id=?1",
            [statement_id],
            |row| row.get(0),
        )
        .map_err(|_| "El resumen no existe.".to_string())?;
    let items = connection
        .prepare("SELECT id,transaction_id FROM finance_credit_card_statement_items WHERE statement_id=?1")
        .map_err(|error| error.to_string())?
        .query_map([statement_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let timestamp = now();
    for transaction_id in items.iter().filter_map(|(_, transaction_id)| transaction_id.as_deref()) {
        connection
            .execute(
                "UPDATE finance_service_occurrences SET transaction_id=NULL,paid_amount=NULL,status='pending',updated_at=?1
                 WHERE transaction_id=?2",
                params![timestamp, transaction_id],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE finance_transactions SET service_id=NULL,updated_at=?1 WHERE id=?2",
                params![timestamp, transaction_id],
            )
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute(
            "DELETE FROM finance_credit_card_statement_items WHERE statement_id=?1",
            [statement_id],
        )
        .map_err(|error| error.to_string())?;
    for (item_id, transaction_id) in &items {
        if let Some(transaction_id) = transaction_id {
            retire_stale_card_statement_transaction(connection, item_id, transaction_id, &timestamp)?;
        }
    }
    connection
        .execute("DELETE FROM finance_credit_card_statements WHERE id=?1", [statement_id])
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE finance_source_artifacts SET deleted_at=?1,content_hash=NULL WHERE id=?2",
            params![timestamp, artifact_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "Se borró el resumen con sus {} línea(s); los tickets que había pagado vuelven a esperar un resumen.",
        items.len()
    ))
}

pub(crate) fn persist_card_reconciliation(
    transaction: &rusqlite::Connection,
    source_reference: Option<&str>,
    raw_source: Option<&str>,
    artifact_id: &str,
    reconciliation: &CardServiceReconciliation,
    actor_library_user_id: &str,
    source: &str,
) -> Result<(), String> {
    for assignment in &reconciliation.assignments {
        let (service_currency, expected_amount): (String, String) = transaction
            .query_row(
                "SELECT currency,expected_amount FROM finance_services WHERE id=?1",
                [&assignment.service_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| {
                "El servicio de la reconciliación no existe o está inactivo.".to_string()
            })?;
        if service_currency != assignment.currency {
            return Err("La moneda del consumo no coincide con el servicio.".into());
        }
        let transaction_state: (String, String, String, String, Option<String>) = transaction
            .query_row(
                "SELECT transaction_type,status,amount,currency,service_id FROM finance_transactions
                 WHERE id=?1 AND deleted_at IS NULL",
                [&assignment.transaction_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|_| "La transacción de la reconciliación no existe.".to_string())?;
        if transaction_state.0 != "expense"
            || !matches!(transaction_state.1.as_str(), "confirmed" | "corrected")
            || parse_cents(&transaction_state.2) != parse_cents(&assignment.amount)
            || transaction_state.3 != assignment.currency
        {
            return Err(
                "La transacción de la reconciliación cambió o no es un gasto confirmado.".into(),
            );
        }
        if transaction_state
            .4
            .as_deref()
            .is_some_and(|service_id| service_id != assignment.service_id)
        {
            return Err("La transacción ya está vinculada a otro servicio.".into());
        }
        let old: Option<(String, Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT id,paid_amount,transaction_id FROM finance_service_occurrences
                 WHERE service_id=?1 AND period=?2",
                params![assignment.service_id, assignment.period],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((occurrence_id, paid_amount, old_transaction_id)) = &old {
            let is_same = old_transaction_id.as_deref() == Some(assignment.transaction_id.as_str())
                && paid_amount
                    .as_deref()
                    .is_some_and(|value| parse_cents(value) == parse_cents(&assignment.amount));
            if !is_same && (paid_amount.is_some() || old_transaction_id.is_some()) {
                return Err("La ocurrencia destino ya está ocupada por otro pago.".into());
            }
            if !is_same {
                let version: i64 = transaction
                    .query_row(
                        "SELECT COALESCE(MAX(version_number),0)+1
                         FROM finance_service_occurrence_versions WHERE occurrence_id=?1",
                        [occurrence_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                transaction
                    .execute(
                        "INSERT INTO finance_service_occurrence_versions(
                         id,occurrence_id,version_number,expected_amount,paid_amount,effective_date,
                         status,transaction_id,artifact_id,source_reference,raw_source,
                         actor_library_user_id,source,reason,created_at)
                         SELECT ?1,id,?2,expected_amount,paid_amount,effective_date,status,
                                transaction_id,artifact_id,source_reference,raw_source,
                                actor_library_user_id,source,?3,?4
                         FROM finance_service_occurrences WHERE id=?5",
                        params![
                            Uuid::new_v4().to_string(),
                            version,
                            "Resumen de tarjeta reconciliado",
                            now(),
                            occurrence_id
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction
            .execute(
                "UPDATE finance_transactions SET service_id=?1,updated_at=?2
                 WHERE id=?3 AND deleted_at IS NULL",
                params![assignment.service_id, now(), assignment.transaction_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE finance_purchases SET service_id=?1,updated_at=?2
                 WHERE transaction_id=?3",
                params![assignment.service_id, now(), assignment.transaction_id],
            )
            .map_err(|error| error.to_string())?;
        let occurrence_id = old
            .as_ref()
            .map(|value| value.0.clone())
            .unwrap_or_else(|| {
                format!(
                    "service-occurrence:{}:{}",
                    assignment.service_id, assignment.period
                )
            });
        transaction
            .execute(
                "INSERT INTO finance_service_occurrences(
                 id,service_id,period,expected_amount,paid_amount,effective_date,status,
                 transaction_id,artifact_id,source_reference,raw_source,actor_library_user_id,
                 source,created_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,'accepted',?7,?8,?9,?10,?11,?12,?13,?13)
                 ON CONFLICT(service_id,period) DO UPDATE SET
                 expected_amount=excluded.expected_amount,paid_amount=excluded.paid_amount,
                 effective_date=excluded.effective_date,status=excluded.status,
                 transaction_id=excluded.transaction_id,artifact_id=excluded.artifact_id,
                 source_reference=excluded.source_reference,raw_source=excluded.raw_source,
                 actor_library_user_id=excluded.actor_library_user_id,source=excluded.source,
                 updated_at=excluded.updated_at",
                params![
                    occurrence_id,
                    assignment.service_id,
                    assignment.period,
                    expected_amount,
                    assignment.amount,
                    assignment.purchase_date,
                    assignment.transaction_id,
                    artifact_id,
                    source_reference,
                    raw_source,
                    actor_library_user_id,
                    source,
                    now()
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Saves a credit-card statement. Loading it means it was paid: every
/// consumption, fee, interest and tax counts in the month it is due, and the
/// total is what was paid for the card, never another expense. Card-unpaid
/// expenses (tickets or loose expenses) that a line pays become that line's
/// expense; installment lines keep their plan up to date.
pub fn finance_save_credit_card_statement(
    app: crate::host::AppHandle,
    payload: SaveCreditCardStatementPayload,
) -> FinanceCommandResult<SavedCreditCardStatement> {
    let mut connection = validate_context(&payload.context, &app)?;
    let saved = store_credit_card_statement(&mut connection, &payload.statement, &payload.context)?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(saved)
}

pub(crate) fn store_credit_card_statement(
    connection: &mut rusqlite::Connection,
    statement: &CreditCardStatement,
    context: &FinanceContext,
) -> FinanceCommandResult<SavedCreditCardStatement> {
    validate_credit_card_statement(statement)?;
    if statement
        .source_reference
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
        || statement
            .source_reference
            .as_deref()
            .is_some_and(|reference| reference.len() > 2_048)
    {
        return Err("El resumen requiere la referencia al archivo original.".into());
    }
    let account_type: Option<String> = connection
        .query_row(
            "SELECT account_type FROM finance_accounts WHERE id=?1 AND active=1",
            [&statement.account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(account_type) = account_type else {
        return Err("La cuenta de tarjeta no existe o está inactiva.".into());
    };
    if account_type != "credit_card" {
        return Err("El resumen debe asociarse a una cuenta de tipo tarjeta de crédito.".into());
    }
    for category_id in statement.items.iter().filter_map(|item| item.category_id.as_deref()) {
        let kind: String = connection
            .query_row(
                "SELECT kind FROM finance_categories WHERE id=?1 AND active=1",
                [category_id],
                |row| row.get(0),
            )
            .map_err(|_| "La categoría de una línea no existe o está inactiva.".to_string())?;
        if kind != "expense" {
            return Err("Las líneas del resumen solo usan categorías de gasto.".into());
        }
    }
    let duplicate: Option<String> = connection
        .query_row(
            "SELECT id FROM finance_credit_card_statements WHERE account_id=?1 AND period=?2 AND currency=?3 AND id<>?4",
            params![statement.account_id, statement.period, statement.currency, statement.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if duplicate.is_some() {
        return Err("El resumen de tarjeta ya fue registrado anteriormente.".into());
    }

    let timestamp = now();
    let artifact_candidate_id = format!("card-statement:{}", statement.id);
    let content_hash = format!(
        "card-statement:{}:{}:{}",
        statement.account_id, statement.period, statement.currency
    );
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT OR IGNORE INTO finance_categories(id,name,kind,active,description,created_at,updated_at)
         VALUES(?1,'Cargos de tarjeta','expense',1,'Cargos, intereses e impuestos de las tarjetas de crédito.',?2,?2)",
        params![CARD_CHARGES_CATEGORY, timestamp],
    ).map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT OR IGNORE INTO finance_source_artifacts(id,source_type,reference,raw_text,content_hash,created_at)
         VALUES(?1,'credit_card_statement',?2,?3,?4,?5)",
        params![artifact_candidate_id, statement.source_reference, statement.raw_extraction, content_hash, timestamp],
    ).map_err(|error| error.to_string())?;
    let artifact_id: String = transaction
        .query_row(
            "SELECT id FROM finance_source_artifacts WHERE content_hash=?1",
            [&content_hash],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO finance_credit_card_statements(
            id,account_id,issuer,card_last_four,period,closing_date,due_date,currency,
            previous_balance,payments_amount,credits_amount,purchases_amount,fees_amount,
            interest_amount,taxes_amount,total_due,minimum_payment,source_artifact_id,
            validation_status,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?20)
         ON CONFLICT(id) DO UPDATE SET
             account_id=excluded.account_id,issuer=excluded.issuer,card_last_four=excluded.card_last_four,
             period=excluded.period,closing_date=excluded.closing_date,due_date=excluded.due_date,
             currency=excluded.currency,previous_balance=excluded.previous_balance,
             payments_amount=excluded.payments_amount,credits_amount=excluded.credits_amount,
             purchases_amount=excluded.purchases_amount,fees_amount=excluded.fees_amount,
             interest_amount=excluded.interest_amount,taxes_amount=excluded.taxes_amount,
             total_due=excluded.total_due,minimum_payment=excluded.minimum_payment,
             source_artifact_id=excluded.source_artifact_id,validation_status=excluded.validation_status,
             updated_at=excluded.updated_at",
            params![
                statement.id,
                statement.account_id,
                statement.issuer,
                statement.card_last_four,
                statement.period,
                statement.closing_date,
                statement.due_date,
                statement.currency,
                statement.previous_balance,
                statement.payments_amount,
                statement.credits_amount,
                statement.purchases_amount,
                statement.fees_amount,
                statement.interest_amount,
                statement.taxes_amount,
                statement.total_due,
                statement.minimum_payment,
                artifact_id,
                statement.status,
                timestamp
            ],
        )
        .map_err(|error| error.to_string())?;

    let mut saved_statement = statement.clone();
    let mut outcome = LinkOutcome::default();
    let mut matched_existing_transactions = 0_usize;
    let mut created_transactions = 0_usize;
    let previous_items = transaction
        .prepare(
            "SELECT id,transaction_id FROM finance_credit_card_statement_items
             WHERE statement_id=?1",
        )
        .map_err(|error| error.to_string())?
        .query_map([&statement.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let previous_transactions = previous_items
        .iter()
        .filter_map(|(item_id, transaction_id)| transaction_id.clone().map(|id| (item_id.clone(), id)))
        .collect::<BTreeMap<_, _>>();
    // An expense a line keeps: same amount and currency, and no other line uses it.
    let usable = |candidate: &str, item: &CreditCardStatementItem| -> Result<bool, String> {
        let amount: Option<String> = transaction
            .query_row(
                "SELECT amount FROM finance_transactions
                 WHERE id=?1 AND transaction_type='expense' AND currency=?2
                   AND status IN ('confirmed','corrected',?3) AND deleted_at IS NULL
                   AND NOT EXISTS (SELECT 1 FROM finance_credit_card_statement_items other
                                   WHERE other.transaction_id=?1 AND other.id<>?4)",
                params![candidate, item.currency, CARD_UNPAID, item.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(amount.is_some_and(|amount| parse_cents(&amount) == parse_cents(&item.amount)))
    };
    for item in &mut saved_statement.items {
        let creates_expense = statement.status != "pending"
            && matches!(
                item.item_type.as_str(),
                "purchase" | "fee" | "interest" | "tax"
            );
        let line = CardLine {
            item_id: item.id.clone(),
            account_id: statement.account_id.clone(),
            currency: item.currency.clone(),
            purchase_date: item.purchase_date.clone(),
            description: item.description.clone(),
            amount: parse_cents(&item.amount).unwrap_or_default(),
            installment_number: item.installment_number.map(i64::from),
            installment_count: item.installment_count.map(i64::from),
            transaction_id: None,
            due_date: statement.due_date.clone(),
        };
        let mut reused = None;
        let mut created = false;
        let linked_transaction = if !creates_expense {
            None
        } else if let Some(kept) = previous_transactions
            .get(&item.id)
            .filter(|candidate| usable(candidate, item).unwrap_or(false))
            .or(item.transaction_id.as_ref().filter(|candidate| usable(candidate, item).unwrap_or(false)))
            .cloned()
        {
            // A line saved before, or one naming its expense, keeps it and
            // counts it in the month the statement is due.
            transaction
                .execute(
                    "UPDATE finance_transactions SET status='confirmed',effective_date=?1,
                            purchase_date=COALESCE(purchase_date,?2),updated_at=?3
                     WHERE id=?4",
                    params![statement.due_date, item.purchase_date, timestamp, kept],
                )
                .map_err(|error| error.to_string())?;
            matched_existing_transactions += 1;
            Some(kept)
        } else {
            let decision = if item.item_type == "purchase" {
                finance_matching::match_line_before_save(&transaction, &line)?
            } else {
                LineMatch::None
            };
            match decision {
                LineMatch::Reuse(unpaid_id) => {
                    matched_existing_transactions += 1;
                    reused = Some(unpaid_id.clone());
                    Some(unpaid_id)
                }
                LineMatch::None | LineMatch::Later => {
                    let merchant_id = if item.item_type == "purchase" {
                        find_merchant_for_descriptor(&transaction, &item.description)?
                    } else {
                        None
                    };
                    let category_id = if item.item_type == "purchase" {
                        match (item.category_id.clone(), merchant_id.as_deref()) {
                            (Some(category_id), _) => Some(category_id),
                            (None, Some(merchant)) => last_merchant_category(&transaction, merchant)?,
                            (None, None) => None,
                        }
                    } else {
                        Some(CARD_CHARGES_CATEGORY.to_string())
                    };
                    created = true;
                    created_transactions += 1;
                    Some(finance_matching::insert_line_transaction(
                        &transaction,
                        &line,
                        &item.item_type,
                        category_id.as_deref(),
                        merchant_id.as_deref(),
                        &artifact_id,
                    )?)
                }
            }
        };
        item.transaction_id = linked_transaction.clone();
        transaction.execute(
            "INSERT INTO finance_credit_card_statement_items(id,statement_id,transaction_id,purchase_date,description,amount,currency,item_type,installment_number,installment_count,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET statement_id=excluded.statement_id,
                 transaction_id=excluded.transaction_id,purchase_date=excluded.purchase_date,
                 description=excluded.description,amount=excluded.amount,currency=excluded.currency,
                 item_type=excluded.item_type,installment_number=excluded.installment_number,
                 installment_count=excluded.installment_count",
            params![item.id,statement.id,linked_transaction,item.purchase_date,item.description,item.amount,item.currency,item.item_type,item.installment_number,item.installment_count,timestamp],
        ).map_err(|error| error.to_string())?;
        if let Some(unpaid_id) = reused {
            finance_matching::apply_reuse(&transaction, &unpaid_id, &line, &mut outcome)?;
        } else if created && item.item_type == "purchase" {
            finance_matching::link_line_after_save(&transaction, &item.id, &mut outcome)?;
        }
        if creates_expense && item.item_type == "purchase" {
            finance_matching::link_installment_line(&transaction, &item.id, &mut outcome)?;
        }
    }
    let current_item_ids = saved_statement
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let current_item_transactions = saved_statement
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.transaction_id.as_deref()))
        .collect::<BTreeMap<_, _>>();
    for (item_id, transaction_id) in &previous_items {
        if !current_item_ids.contains(item_id.as_str()) {
            transaction
                .execute(
                    "DELETE FROM finance_credit_card_statement_items WHERE statement_id=?1 AND id=?2",
                    params![statement.id, item_id],
                )
                .map_err(|error| error.to_string())?;
        }
        let Some(old_transaction_id) = transaction_id else {
            continue;
        };
        let replaced = current_item_transactions
            .get(item_id.as_str())
            .copied()
            .flatten()
            != Some(old_transaction_id.as_str());
        if replaced {
            retire_stale_card_statement_transaction(&transaction, item_id, old_transaction_id, &timestamp)?;
        }
    }
    let reconciliation_input =
        load_card_reconciliation_input(&transaction, statement, &saved_statement.items)?;
    let mut reconciliation = reconcile_card_service_consumption(&reconciliation_input);
    persist_card_reconciliation(
        &transaction,
        statement.source_reference.as_deref(),
        statement.raw_extraction.as_deref(),
        &artifact_id,
        &reconciliation,
        &context.actor_library_user_id,
        &context.source,
    )?;
    service_review_items(&transaction, &reconciliation_input, &reconciliation, &mut outcome)?;
    if !reconciliation.assignments.is_empty() {
        reconciliation.status = if reconciliation.ambiguous_groups.is_empty() {
            "applied".into()
        } else {
            "partial-applied".into()
        };
    }
    let persisted_created_at: Option<String> = transaction
        .query_row(
            "SELECT created_at FROM finance_credit_card_statements WHERE id=?1",
            [&statement.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    saved_statement.created_at = persisted_created_at;
    let occurrences = reconciliation
        .assignments
        .iter()
        .map(|assignment| {
            occurrence_from_connection(connection, &assignment.service_id, &assignment.period)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SavedCreditCardStatement {
        statement: saved_statement,
        matched_existing_transactions,
        created_transactions,
        reconciliation,
        occurrences,
        outcome,
    })
}

/// Asks which service a card consumption belongs to when its description
/// names more than one. Consumptions that name no service are ordinary
/// purchases and are not asked about.
fn service_review_items(
    connection: &rusqlite::Connection,
    input: &ReconciliationInput,
    reconciliation: &CardServiceReconciliation,
    outcome: &mut LinkOutcome,
) -> Result<(), String> {
    for group in reconciliation
        .ambiguous_groups
        .iter()
        .filter(|group| group.reason.code == "multiple-service-match")
    {
        let Some(line) = group
            .line_ids
            .first()
            .and_then(|line_id| input.lines.iter().find(|line| &line.id == line_id))
        else {
            continue;
        };
        let mut options = Vec::new();
        for service_id in &group.candidate_service_ids {
            if let Some(service) = input.services.iter().find(|service| &service.id == service_id) {
                options.push(ReviewOption {
                    id: format!("service:{}", service.id),
                    label: service.name.clone(),
                });
            }
        }
        options.push(ReviewOption {
            id: "none".into(),
            label: "Ninguno: no es un servicio".into(),
        });
        outcome.review_items.extend(finance_matching::create_review_item(
            connection,
            "service-card-line",
            &line.id,
            serde_json::json!({ "lineId": line.id, "statementId": input.statement_id }),
            format!(
                "¿De qué servicio es el consumo «{}» del {} por {} {}?",
                line.description, line.purchase_date, line.currency, line.amount
            ),
            options,
        )?);
    }
    Ok(())
}

fn load_statement(
    connection: &rusqlite::Connection,
    statement_id: &str,
) -> Result<CreditCardStatement, String> {
    let mut statement = connection
        .query_row(
            "SELECT s.id,s.account_id,s.issuer,s.card_last_four,s.period,s.closing_date,s.due_date,
                s.currency,s.previous_balance,s.payments_amount,s.credits_amount,s.purchases_amount,
                s.fees_amount,s.interest_amount,s.taxes_amount,s.total_due,s.minimum_payment,
                s.validation_status,s.created_at,a.reference,a.raw_text
             FROM finance_credit_card_statements s
             LEFT JOIN finance_source_artifacts a ON a.id=s.source_artifact_id
             WHERE s.id=?1",
            [statement_id],
            statement_from_row,
        )
        .map_err(|_| "El resumen no existe.".to_string())?;
    statement.items = statement_items(connection, statement_id)?;
    Ok(statement)
}

/// Review items for the service consumptions of a stored statement.
pub(crate) fn review_service_lines(
    connection: &rusqlite::Connection,
    statement_id: &str,
    outcome: &mut LinkOutcome,
) -> Result<(), String> {
    let statement = load_statement(connection, statement_id)?;
    let input = load_card_reconciliation_input(connection, &statement, &statement.items)?;
    let reconciliation = reconcile_card_service_consumption(&input);
    service_review_items(connection, &input, &reconciliation, outcome)
}

/// Records a card consumption as the payment of a service in the
/// statement's month, as the person chose.
pub(crate) fn assign_card_line_to_service(
    connection: &rusqlite::Connection,
    item_id: &str,
    service_id: &str,
    actor_library_user_id: &str,
    source: &str,
) -> Result<(), String> {
    let (statement_id, transaction_id, purchase_date, amount, currency): (String, Option<String>, String, String, String) =
        connection
            .query_row(
                "SELECT statement_id,transaction_id,purchase_date,amount,currency
                 FROM finance_credit_card_statement_items WHERE id=?1",
                [item_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .map_err(|_| "La línea del resumen no existe.".to_string())?;
    let transaction_id =
        transaction_id.ok_or_else(|| "La línea del resumen no tiene gasto.".to_string())?;
    let statement = load_statement(connection, &statement_id)?;
    let artifact_id: String = connection
        .query_row(
            "SELECT source_artifact_id FROM finance_credit_card_statements WHERE id=?1",
            [&statement_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let assignment = crate::finance_reconciliation::CardServiceAssignment {
        statement_id,
        line_id: item_id.to_string(),
        service_id: service_id.to_string(),
        transaction_id,
        purchase_date,
        period: statement.period.clone(),
        amount,
        currency,
        assignment_status: "new".into(),
        evidence: serde_json::json!({ "matching": "chosen-by-person", "lineId": item_id }),
    };
    persist_card_reconciliation(
        connection,
        statement.source_reference.as_deref(),
        statement.raw_extraction.as_deref(),
        &artifact_id,
        &CardServiceReconciliation {
            status: "ready".into(),
            assignments: vec![assignment],
            ambiguous_groups: Vec::new(),
            reasons: Vec::new(),
        },
        actor_library_user_id,
        source,
    )
}

fn statement_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CreditCardStatement> {
    Ok(CreditCardStatement {
        id: row.get(0)?,
        account_id: row.get(1)?,
        issuer: row.get(2)?,
        card_last_four: row.get(3)?,
        period: row.get(4)?,
        closing_date: row.get(5)?,
        due_date: row.get(6)?,
        currency: row.get(7)?,
        previous_balance: row.get(8)?,
        payments_amount: row.get(9)?,
        credits_amount: row.get(10)?,
        purchases_amount: row.get(11)?,
        fees_amount: row.get(12)?,
        interest_amount: row.get(13)?,
        taxes_amount: row.get(14)?,
        total_due: row.get(15)?,
        minimum_payment: row.get(16)?,
        status: row.get(17)?,
        created_at: row.get(18)?,
        source_reference: row.get(19)?,
        raw_extraction: row.get(20)?,
        items: Vec::new(),
    })
}

fn statement_items(
    connection: &rusqlite::Connection,
    statement_id: &str,
) -> Result<Vec<CreditCardStatementItem>, String> {
    let mut item_query = connection
        .prepare(
            "SELECT i.id,i.purchase_date,i.description,i.amount,i.currency,i.item_type,
                    i.installment_number,i.installment_count,i.transaction_id,t.category_id
             FROM finance_credit_card_statement_items i
             LEFT JOIN finance_transactions t ON t.id=i.transaction_id
             WHERE i.statement_id=?1 ORDER BY i.purchase_date,i.id",
        )
        .map_err(|error| error.to_string())?;
    let items = item_query
        .query_map([statement_id], |row| {
            Ok(CreditCardStatementItem {
                id: row.get(0)?,
                purchase_date: row.get(1)?,
                description: row.get(2)?,
                amount: row.get(3)?,
                currency: row.get(4)?,
                item_type: row.get(5)?,
                installment_number: row.get(6)?,
                installment_count: row.get(7)?,
                transaction_id: row.get(8)?,
                category_id: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(items)
}

pub fn finance_list_credit_card_statements(
    app: crate::host::AppHandle,
    payload: ListPeriodPayload,
) -> FinanceCommandResult<Vec<CreditCardStatement>> {
    let connection = validate_context(&payload.context, &app)?;
    let mut statement_query = connection
        .prepare(
            "SELECT s.id,s.account_id,s.issuer,s.card_last_four,s.period,s.closing_date,s.due_date,
                s.currency,s.previous_balance,s.payments_amount,s.credits_amount,s.purchases_amount,
                s.fees_amount,s.interest_amount,s.taxes_amount,s.total_due,s.minimum_payment,
                 s.validation_status,s.created_at,a.reference,a.raw_text
         FROM finance_credit_card_statements s
         JOIN finance_source_artifacts a ON a.id=s.source_artifact_id
         WHERE (?1 IS NULL OR s.period>=?1) AND (?2 IS NULL OR s.period<=?2)
         ORDER BY s.period DESC,s.due_date DESC",
        )
        .map_err(|error| error.to_string())?;
    let mut statements = statement_query
        .query_map(params![payload.from, payload.to], statement_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement_query);
    for statement in &mut statements {
        statement.items = statement_items(&connection, &statement.id)?;
    }
    Ok(statements)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstallmentPlan {
    pub id: String,
    pub account_id: String,
    pub merchant_name: String,
    pub description: String,
    pub purchase_date: String,
    pub currency: String,
    pub total_amount: String,
    pub installment_count: u16,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveInstallmentPayload {
    pub context: FinanceContext,
    pub plan: InstallmentPlan,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installment {
    pub id: String,
    pub plan_id: String,
    pub installment_number: u16,
    pub due_date: String,
    pub amount: String,
    pub status: String,
}

/// Saves a plan of installments as a schedule. Installments are not
/// expenses: each one becomes an expense when a statement line pays it.
pub fn finance_save_installment_plan(
    app: crate::host::AppHandle,
    payload: SaveInstallmentPayload,
) -> FinanceCommandResult<Vec<Installment>> {
    let plan = &payload.plan;
    if plan.id.trim().is_empty()
        || plan.installment_count == 0
        || plan.installment_count > 120
        || !valid_date(&plan.purchase_date)
        || !matches!(plan.currency.as_str(), "ARS" | "USD")
        || plan.description.trim().is_empty()
    {
        return Err("El plan de cuotas requiere descripción, fecha, moneda y entre 1 y 120 cuotas.".into());
    }
    let total = ensure_money(&plan.total_amount, "El total")?;
    if total <= 0 {
        return Err("El total debe ser positivo.".into());
    }
    let mut connection = validate_context(&payload.context, &app)?;
    let (account_type, account_currency): (String, String) = connection
        .query_row(
            "SELECT account_type,currency FROM finance_accounts WHERE id=?1 AND active=1",
            [&plan.account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "La cuenta del plan no existe o está inactiva.".to_string())?;
    if account_type != "credit_card" && account_currency != plan.currency {
        return Err("La moneda del plan no coincide con la cuenta.".into());
    }
    let timestamp = now();
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut outcome = LinkOutcome::default();
    let merchant_id = if plan.merchant_name.trim().is_empty() {
        None
    } else {
        Some(finance_matching::resolve_merchant(&transaction, &plan.merchant_name, &mut outcome)?)
    };
    transaction.execute("INSERT INTO finance_installment_plans(id,account_id,merchant_id,description,purchase_date,currency,total_amount,installment_count,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9) ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,merchant_id=excluded.merchant_id,description=excluded.description,purchase_date=excluded.purchase_date,currency=excluded.currency,total_amount=excluded.total_amount,installment_count=excluded.installment_count,updated_at=excluded.updated_at",params![plan.id,plan.account_id,merchant_id,plan.description.trim(),plan.purchase_date,plan.currency,plan.total_amount,plan.installment_count,timestamp]).map_err(|error|error.to_string())?;
    let count = i128::from(plan.installment_count);
    let (base, remainder) = (total / count, total % count);
    for number in 1..=plan.installment_count {
        let amount = base + i128::from(i128::from(number) <= remainder);
        let due_date = add_months(&plan.purchase_date[..10], i32::from(number) - 1)?;
        // A paid installment keeps its expense and state; only the schedule changes.
        transaction.execute("INSERT INTO finance_installments(id,plan_id,installment_number,due_date,amount,status,transaction_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'pending',NULL,?6,?6) ON CONFLICT(id) DO UPDATE SET due_date=excluded.due_date,amount=excluded.amount,updated_at=excluded.updated_at",params![format!("{}:{}", plan.id, number),plan.id,number,due_date,format_cents(amount),timestamp]).map_err(|error|error.to_string())?;
    }
    transaction
        .execute(
            "DELETE FROM finance_installments WHERE plan_id=?1 AND installment_number>?2 AND transaction_id IS NULL",
            params![plan.id, plan.installment_count],
        )
        .map_err(|error| error.to_string())?;
    let installments = installments_of(&transaction, Some(&plan.id))?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    sync_context(&payload.context, &app)?;
    Ok(installments)
}

/// A plan with what is left to pay: the pending installments, the next due
/// date and their total.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallmentPlanSummary {
    #[serde(flatten)]
    pub plan: InstallmentPlan,
    pub pending_count: i64,
    pub next_due_date: Option<String>,
    pub remaining_amount: String,
}

pub fn finance_list_installment_plans(
    app: crate::host::AppHandle,
    context: FinanceContext,
) -> FinanceCommandResult<Vec<InstallmentPlanSummary>> {
    let connection = validate_context(&context, &app)?;
    let mut statement = connection
        .prepare(
            "SELECT p.id,p.account_id,COALESCE(m.name,''),p.description,p.purchase_date,p.currency,p.total_amount,p.installment_count
             FROM finance_installment_plans p
             LEFT JOIN finance_merchants m ON m.id=p.merchant_id
             ORDER BY p.purchase_date DESC,p.created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let plans = statement
        .query_map([], |row| {
            Ok(InstallmentPlan {
                id: row.get(0)?,
                account_id: row.get(1)?,
                merchant_name: row.get(2)?,
                description: row.get(3)?,
                purchase_date: row.get(4)?,
                currency: row.get(5)?,
                total_amount: row.get(6)?,
                installment_count: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut summaries = Vec::with_capacity(plans.len());
    for plan in plans {
        let pending = installments_of(&connection, Some(&plan.id))?
            .into_iter()
            .filter(|installment| installment.status == "pending")
            .collect::<Vec<_>>();
        let remaining = pending
            .iter()
            .map(|installment| parse_cents(&installment.amount).unwrap_or_default())
            .sum::<i128>();
        summaries.push(InstallmentPlanSummary {
            pending_count: pending.len() as i64,
            next_due_date: pending.first().map(|installment| installment.due_date.clone()),
            remaining_amount: format_cents(remaining),
            plan,
        });
    }
    Ok(summaries)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInstallmentsPayload {
    pub context: FinanceContext,
    pub plan_id: Option<String>,
}

fn installments_of(
    connection: &rusqlite::Connection,
    plan_id: Option<&str>,
) -> Result<Vec<Installment>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,plan_id,installment_number,due_date,amount,status
             FROM finance_installments
             WHERE (?1 IS NULL OR plan_id=?1)
             ORDER BY due_date,installment_number",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([plan_id], |row| {
            Ok(Installment {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                installment_number: row.get(2)?,
                due_date: row.get(3)?,
                amount: row.get(4)?,
                status: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub fn finance_list_installments(
    app: crate::host::AppHandle,
    payload: ListInstallmentsPayload,
) -> FinanceCommandResult<Vec<Installment>> {
    let connection = validate_context(&payload.context, &app)?;
    Ok(installments_of(&connection, payload.plan_id.as_deref())?)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCatalogPayload {
    pub context: FinanceContext,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductPrice {
    pub merchant_id: Option<String>,
    pub merchant_name: Option<String>,
    pub observed_at: String,
    pub currency: String,
    pub quantity: String,
    pub unit_price: String,
    pub final_amount: String,
}

/// A product with the last price paid at each merchant.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSummary {
    pub id: String,
    pub name: String,
    pub observation_count: i64,
    pub last_observed_at: Option<String>,
    pub prices: Vec<ProductPrice>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MerchantSummary {
    pub id: String,
    pub name: String,
    pub purchase_count: i64,
    pub last_purchase_at: Option<String>,
    pub last_category_id: Option<String>,
}

fn matches_search(search: Option<&str>, name: &str) -> bool {
    let Some(search) = search.map(finance_matching::normalize_text).filter(|value| !value.is_empty()) else {
        return true;
    };
    let name = finance_matching::normalize_text(name);
    search.split_whitespace().all(|word| name.contains(word))
}

pub fn finance_list_products(
    app: crate::host::AppHandle,
    payload: ListCatalogPayload,
) -> FinanceCommandResult<Vec<ProductSummary>> {
    let connection = validate_context(&payload.context, &app)?;
    let limit = payload.limit.unwrap_or(200).clamp(1, 500) as usize;
    let mut statement = connection
        .prepare(
            "SELECT p.id,p.name,COUNT(o.id),MAX(o.observed_at)
             FROM finance_products p
             LEFT JOIN finance_price_observations o
               ON o.product_id=p.id AND o.status IN ('confirmed','corrected')
             GROUP BY p.id ORDER BY MAX(o.observed_at) DESC,p.name",
        )
        .map_err(|error| error.to_string())?;
    let products = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|(_, name, _, _)| matches_search(payload.search.as_deref(), name))
        .take(limit)
        .collect::<Vec<_>>();
    drop(statement);
    let mut prices_query = connection
        .prepare(
            "SELECT o.merchant_id,m.name,o.observed_at,o.currency,o.quantity,o.unit_price,o.final_amount
             FROM finance_price_observations o LEFT JOIN finance_merchants m ON m.id=o.merchant_id
             WHERE o.product_id=?1 AND o.status IN ('confirmed','corrected')
             ORDER BY o.observed_at DESC,o.created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let mut summaries = Vec::with_capacity(products.len());
    for (id, name, observation_count, last_observed_at) in products {
        let mut seen_merchants = BTreeSet::new();
        let prices = prices_query
            .query_map([&id], |row| {
                Ok(ProductPrice {
                    merchant_id: row.get(0)?,
                    merchant_name: row.get(1)?,
                    observed_at: row.get(2)?,
                    currency: row.get(3)?,
                    quantity: row.get(4)?,
                    unit_price: row.get(5)?,
                    final_amount: row.get(6)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|price| seen_merchants.insert(price.merchant_id.clone()))
            .take(10)
            .collect();
        summaries.push(ProductSummary {
            id,
            name,
            observation_count,
            last_observed_at,
            prices,
        });
    }
    Ok(summaries)
}

pub fn finance_list_merchants(
    app: crate::host::AppHandle,
    payload: ListCatalogPayload,
) -> FinanceCommandResult<Vec<MerchantSummary>> {
    let connection = validate_context(&payload.context, &app)?;
    let limit = payload.limit.unwrap_or(200).clamp(1, 500) as usize;
    let mut statement = connection
        .prepare(
            "SELECT m.id,m.name,COUNT(p.id),MAX(p.observed_at),
                    (SELECT t.category_id FROM finance_transactions t
                     WHERE t.merchant_id=m.id AND t.category_id IS NOT NULL AND t.deleted_at IS NULL
                     ORDER BY t.effective_date DESC LIMIT 1)
             FROM finance_merchants m LEFT JOIN finance_purchases p ON p.merchant_id=m.id
             GROUP BY m.id ORDER BY MAX(p.observed_at) DESC,m.name",
        )
        .map_err(|error| error.to_string())?;
    let merchants = statement
        .query_map([], |row| {
            Ok(MerchantSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                purchase_count: row.get(2)?,
                last_purchase_at: row.get(3)?,
                last_category_id: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|merchant| matches_search(payload.search.as_deref(), &merchant.name))
        .take(limit)
        .collect();
    Ok(merchants)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticket_arithmetic_is_exact() {
        let record = PurchaseRecord {
            id: "p".into(),
            account_id: "a".into(),
            category_id: None,
            service_id: None,
            merchant_name: "m".into(),
            observed_at: "2026-08-29".into(),
            currency: "ARS".into(),
            subtotal_amount: "0.30".into(),
            discount_amount: "0.05".into(),
            tax_amount: "0.01".into(),
            total_amount: "0.26".into(),
            status: "confirmed".into(),
            source_reference: None,
            raw_extraction: None,
            content_hash: None,
            items: vec![PurchaseItem {
                id: "i".into(),
                original_description: "x".into(),
                normalized_description: None,
                quantity: "1".into(),
                unit_price: "0.30".into(),
                discount_amount: "0".into(),
                line_total: "0.30".into(),
                category_id: None,
            }],
        };
        let validation = validate_purchase(&record).expect("valid");
        assert!(validation.valid);
        assert_eq!(validation.calculated_total, "0.26");
        assert_eq!(hash_purchase(&record), hash_purchase(&record));
    }

    #[test]
    fn ticket_accepts_informational_tax_already_included_in_lines() {
        let record = PurchaseRecord {
            id: "included-tax".into(),
            account_id: "account".into(),
            category_id: None,
            service_id: None,
            merchant_name: "Comercio".into(),
            observed_at: "2026-08-30".into(),
            currency: "ARS".into(),
            subtotal_amount: "48000".into(),
            discount_amount: "0".into(),
            tax_amount: "8330.58".into(),
            total_amount: "48000".into(),
            status: "confirmed".into(),
            source_reference: None,
            raw_extraction: None,
            content_hash: None,
            items: vec![PurchaseItem {
                id: "line".into(),
                original_description: "Cena".into(),
                normalized_description: None,
                quantity: "1".into(),
                unit_price: "48000".into(),
                discount_amount: "0".into(),
                line_total: "48000".into(),
                category_id: None,
            }],
        };

        let validation = validate_purchase(&record).expect("included tax must validate");
        assert!(validation.valid);
        assert_eq!(validation.calculated_total, "48000.00");
        assert_eq!(validation.discrepancy, "0.00");
    }

    #[test]
    fn ticket_accepts_a_one_cent_fiscal_rounding_adjustment() {
        let record = PurchaseRecord {
            id: "rounded-fuel".into(),
            account_id: "account".into(),
            category_id: None,
            service_id: None,
            merchant_name: "Estacion de servicio".into(),
            observed_at: "2026-08-28".into(),
            currency: "ARS".into(),
            subtotal_amount: "30000.00".into(),
            discount_amount: "0".into(),
            tax_amount: "0".into(),
            total_amount: "30000.01".into(),
            status: "confirmed".into(),
            source_reference: None,
            raw_extraction: Some("Ajuste de redondeo fiscal: 0.01".into()),
            content_hash: None,
            items: vec![PurchaseItem {
                id: "fuel-line".into(),
                original_description: "Gasoil".into(),
                normalized_description: Some("Combustible".into()),
                quantity: "13.8953".into(),
                unit_price: "2159.00".into(),
                discount_amount: "0".into(),
                line_total: "30000.00".into(),
                category_id: None,
            }],
        };

        let validation = validate_purchase(&record).expect("rounding adjustment must validate");
        assert!(validation.valid);
        assert_eq!(validation.calculated_total, "30000.00");
        assert_eq!(validation.discrepancy, "0.01");
    }

    #[test]
    fn ticket_rejects_a_discrepancy_larger_than_one_cent() {
        let mut record = PurchaseRecord {
            id: "invalid-rounding".into(),
            account_id: "account".into(),
            category_id: None,
            service_id: None,
            merchant_name: "Comercio".into(),
            observed_at: "2026-08-28".into(),
            currency: "ARS".into(),
            subtotal_amount: "10.00".into(),
            discount_amount: "0".into(),
            tax_amount: "0".into(),
            total_amount: "10.02".into(),
            status: "confirmed".into(),
            source_reference: None,
            raw_extraction: None,
            content_hash: None,
            items: vec![PurchaseItem {
                id: "line".into(),
                original_description: "Producto".into(),
                normalized_description: None,
                quantity: "1".into(),
                unit_price: "10.00".into(),
                discount_amount: "0".into(),
                line_total: "10.00".into(),
                category_id: None,
            }],
        };

        let validation = validate_purchase(&record).expect("validation result");
        assert!(!validation.valid);
        assert_eq!(validation.discrepancy, "0.02");
        let message = purchase_mismatch(&record, &validation);
        assert!(message.contains("la suma de las líneas da 10.00") && message.contains("el total impreso es 10.02 (diferencia de 0.02)"), "{message}");

        record.total_amount = "9.98".into();
        let validation = validate_purchase(&record).expect("validation result");
        assert!(!validation.valid);
        assert_eq!(validation.discrepancy, "-0.02");
    }

    #[test]
    fn ticket_fingerprint_changes_with_price_observation() {
        let mut first = PurchaseRecord {
            id: "p1".into(),
            account_id: "a".into(),
            category_id: None,
            service_id: None,
            merchant_name: "Mercado".into(),
            observed_at: "2026-08-29".into(),
            currency: "ARS".into(),
            subtotal_amount: "10.00".into(),
            discount_amount: "0".into(),
            tax_amount: "0".into(),
            total_amount: "10.00".into(),
            status: "confirmed".into(),
            source_reference: None,
            raw_extraction: None,
            content_hash: None,
            items: vec![PurchaseItem {
                id: "i1".into(),
                original_description: "Yerba".into(),
                normalized_description: None,
                quantity: "1".into(),
                unit_price: "10.00".into(),
                discount_amount: "0".into(),
                line_total: "10.00".into(),
                category_id: None,
            }],
        };
        let original = hash_purchase(&first);
        first.items[0].unit_price = "11.00".into();
        assert_ne!(original, hash_purchase(&first));
    }
    fn valid_credit_card_statement() -> CreditCardStatement {
        CreditCardStatement {
            id: "statement-2026-08".into(),
            account_id: "card-account".into(),
            issuer: "Banco Notia".into(),
            card_last_four: Some("1234".into()),
            period: "2026-08".into(),
            closing_date: "2026-08-28".into(),
            due_date: "2026-09-08".into(),
            currency: "ARS".into(),
            previous_balance: "1000".into(),
            payments_amount: "1000".into(),
            credits_amount: "100".into(),
            purchases_amount: "2000".into(),
            fees_amount: "100".into(),
            interest_amount: "50".into(),
            taxes_amount: "200".into(),
            total_due: "2250".into(),
            minimum_payment: Some("500".into()),
            status: "confirmed".into(),
            created_at: None,
            source_reference: Some("telegram-photo:file-1".into()),
            raw_extraction: None,
            items: [
                ("purchase", "Compra", "2000"),
                ("fee", "Cargo", "100"),
                ("interest", "Interés", "50"),
                ("tax", "Impuesto", "200"),
                ("payment", "Pago", "1000"),
                ("credit", "Crédito", "100"),
            ]
            .into_iter()
            .enumerate()
            .map(
                |(index, (item_type, description, amount))| CreditCardStatementItem {
                    id: format!("line-{index}"),
                    purchase_date: "2026-08-15".into(),
                    description: description.into(),
                    amount: amount.into(),
                    currency: "ARS".into(),
                    item_type: item_type.into(),
                    installment_number: None,
                    installment_count: None,
                    transaction_id: None,
                    category_id: None,
                },
            )
            .collect(),
        }
    }

    #[test]
    fn credit_card_statement_reconciles_lines_without_duplicating_total_due() {
        let statement = valid_credit_card_statement();
        validate_credit_card_statement(&statement).expect("valid statement");
        let expense_lines = statement
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.item_type.as_str(),
                    "purchase" | "fee" | "interest" | "tax"
                )
            })
            .count();
        assert_eq!(expense_lines, 4);
        assert_eq!(statement.items.len() - expense_lines, 2);
    }

    #[test]
    fn credit_card_statement_rejects_an_incoherent_total() {
        let mut statement = valid_credit_card_statement();
        statement.total_due = "2250.02".into();
        assert!(validate_credit_card_statement(&statement)
            .expect_err("incoherent total must fail")
            .contains("total a pagar"));
    }

    #[test]
    fn installments_keep_exact_total() {
        let total = 100_i128;
        let count = 3_i128;
        let values = (1..=count)
            .map(|n| total / count + if n <= total % count { 1 } else { 0 })
            .sum::<i128>();
        assert_eq!(values, total);
    }

    #[test]
    fn confirmed_salary_requires_exact_net_and_valid_concepts() {
        let mut salary = SalaryReceipt {
            id: "salary-2026-08".into(),
            period: "2026-08".into(),
            payment_date: "2026-08-31".into(),
            employer: "Empresa SA".into(),
            gross_amount: "1200000".into(),
            deductions_total: "200000".into(),
            net_amount: "1000000".into(),
            currency: "ARS".into(),
            account_id: "account".into(),
            status: "confirmed".into(),
            signed_document: false,
            created_at: None,
            source_reference: Some("telegram:salary.jpg".into()),
            raw_extraction: Some("recibo".into()),
            concepts: vec![SalaryConcept {
                id: "concept-1".into(),
                name: "Sueldo básico".into(),
                concept_type: "earning".into(),
                amount: "1200000".into(),
            }],
        };

        validate_salary_receipt(&salary).expect("valid salary");
        salary.net_amount = "999999.99".into();
        assert!(validate_salary_receipt(&salary)
            .expect_err("invalid net")
            .contains("El neto debería ser"));
    }

    #[test]
    fn signed_pdf_salary_accepts_authoritative_net_with_payroll_adjustments() {
        let salary = SalaryReceipt {
            id: "salary-signed-pdf".into(),
            period: "2026-08".into(),
            payment_date: "2026-08-31".into(),
            employer: "Empresa SA".into(),
            gross_amount: "7000000".into(),
            deductions_total: "374656.53".into(),
            net_amount: "6500000".into(),
            currency: "ARS".into(),
            account_id: "account".into(),
            status: "confirmed".into(),
            signed_document: true,
            created_at: None,
            source_reference: Some("telegram:recibo-firmado.pdf".into()),
            raw_extraction: Some("Adelanto de aguinaldo. Firmado digitalmente.".into()),
            concepts: vec![],
        };

        validate_salary_receipt(&salary).expect("a signed PDF is authoritative");
    }

    #[test]
    fn salary_rejects_invalid_concept_types() {
        let salary = SalaryReceipt {
            id: "salary-2026-08".into(),
            period: "2026-08".into(),
            payment_date: "2026-08-31".into(),
            employer: "Empresa SA".into(),
            gross_amount: "100".into(),
            deductions_total: "10".into(),
            net_amount: "90".into(),
            currency: "ARS".into(),
            account_id: "account".into(),
            status: "confirmed".into(),
            signed_document: false,
            created_at: None,
            source_reference: None,
            raw_extraction: None,
            concepts: vec![SalaryConcept {
                id: "concept-1".into(),
                name: "Concepto".into(),
                concept_type: "unknown".into(),
                amount: "100".into(),
            }],
        };

        assert!(validate_salary_receipt(&salary)
            .expect_err("invalid concept")
            .contains("earning o deduction"));
    }

    #[test]
    fn salary_evolution_is_absolute_and_percentage_per_currency() {
        let salary =
            |id: &str, period: &str, gross: &str, deductions: &str, net: &str| SalaryReceipt {
                id: id.into(),
                period: period.into(),
                payment_date: format!("{period}-28"),
                employer: "Notia".into(),
                gross_amount: gross.into(),
                deductions_total: deductions.into(),
                net_amount: net.into(),
                currency: "ARS".into(),
                account_id: "account".into(),
                status: "confirmed".into(),
                signed_document: false,
                created_at: None,
                source_reference: None,
                raw_extraction: None,
                concepts: Vec::new(),
            };
        let rows = build_salary_evolution(vec![
            salary("s1", "2026-07", "1000", "100", "900"),
            salary("s2", "2026-08", "1100", "110", "990"),
        ])
        .expect("evolution");
        assert_eq!(rows[0].net_change, "90.00");
        assert_eq!(rows[0].gross_change, "100.00");
        assert_eq!(rows[0].deductions_change, "10.00");
        assert_eq!(rows[0].net_change_percent.as_deref(), Some("10.00"));
    }

    #[test]
    fn document_load_timestamp_is_camel_case_and_read_only_on_input() {
        let salary = SalaryReceipt {
            id: "salary".into(),
            period: "2026-08".into(),
            payment_date: "2026-08-31".into(),
            employer: "Empresa SA".into(),
            gross_amount: "100".into(),
            deductions_total: "10".into(),
            net_amount: "90".into(),
            currency: "ARS".into(),
            account_id: "account".into(),
            status: "confirmed".into(),
            signed_document: false,
            created_at: Some("1767225600".into()),
            source_reference: None,
            raw_extraction: None,
            concepts: vec![],
        };
        let serialized = serde_json::to_value(&salary).expect("salary DTO serializes");
        assert_eq!(serialized["createdAt"], "1767225600");

        let payload: SaveSalaryPayload = serde_json::from_value(serde_json::json!({
            "context": {
                "libraryPath": "library",
                "actorLibraryUserId": "user-owner",
                "source": "app"
            },
            "salary": {
                "id": "salary",
                "period": "2026-08",
                "paymentDate": "2026-08-31",
                "employer": "Empresa SA",
                "grossAmount": "100",
                "deductionsTotal": "10",
                "netAmount": "90",
                "currency": "ARS",
                "accountId": "account",
                "status": "confirmed",
                "createdAt": "client-controlled-value",
                "concepts": []
            }
        }))
        .expect("salary payload deserializes");
        assert_eq!(payload.salary.created_at, None);

        let statement = valid_credit_card_statement();
        let serialized = serde_json::to_value(&statement).expect("statement DTO serializes");
        assert_eq!(serialized["createdAt"], serde_json::Value::Null);
        let payload: SaveCreditCardStatementPayload = serde_json::from_value(serde_json::json!({
            "context": {
                "libraryPath": "library",
                "actorLibraryUserId": "user-owner",
                "source": "app"
            },
            "statement": {
                "id": "statement",
                "accountId": "card-account",
                "issuer": "Banco Notia",
                "period": "2026-08",
                "closingDate": "2026-08-28",
                "dueDate": "2026-09-08",
                "currency": "ARS",
                "previousBalance": "1000",
                "paymentsAmount": "1000",
                "creditsAmount": "100",
                "purchasesAmount": "2000",
                "feesAmount": "100",
                "interestAmount": "50",
                "taxesAmount": "200",
                "totalDue": "2250",
                "status": "confirmed",
                "createdAt": "client-controlled-value",
                "items": []
            }
        }))
        .expect("statement payload deserializes");
        assert_eq!(payload.statement.created_at, None);
    }

    #[test]
    fn card_reconciliation_persistence_is_idempotent_and_versions_before_replacement() {
        let mut connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection
            .execute(
                "INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at)
                 VALUES('card','Tarjeta','credit_card','ARS','0',1,'now','now')",
                [],
            )
            .expect("card fixture");
        connection
            .execute(
                "INSERT INTO finance_transactions(id,transaction_type,amount,currency,effective_date,account_id,description,source,status,created_at,updated_at)
                 VALUES('card-line','expense','150.00','ARS','2026-08-15','card','Internet','credit_card_statement','confirmed','now','now')",
                [],
            )
            .expect("transaction fixture");
        connection
            .execute(
                "INSERT INTO finance_services(id,name,normalized_name,category_id,currency,expected_amount,modality,active,created_at,updated_at)
                 VALUES('internet','Internet','internet','default-expense-services','ARS','100.00','fixed',1,'now','now')",
                [],
            )
            .expect("service fixture");
        connection
            .execute(
                "INSERT INTO finance_source_artifacts(id,source_type,reference,created_at)
                 VALUES('statement-artifact','credit_card_statement','statement.pdf','now')",
                [],
            )
            .expect("artifact fixture");
        connection
            .execute(
                "INSERT INTO finance_service_occurrences(id,service_id,period,expected_amount,status,source,created_at,updated_at)
                 VALUES('occurrence','internet','2026-08','100.00','pending','app','original','original')",
                [],
            )
            .expect("occurrence fixture");
        let reconciliation = CardServiceReconciliation {
            status: "ready".into(),
            assignments: vec![crate::finance_reconciliation::CardServiceAssignment {
                statement_id: "statement".into(),
                line_id: "line".into(),
                service_id: "internet".into(),
                transaction_id: "card-line".into(),
                purchase_date: "2026-08-15".into(),
                period: "2026-08".into(),
                amount: "150.00".into(),
                currency: "ARS".into(),
                assignment_status: "new".into(),
                evidence: serde_json::json!({"lineId":"line"}),
            }],
            ambiguous_groups: vec![],
            reasons: vec![],
        };
        let statement = CreditCardStatement {
            id: "statement".into(),
            account_id: "card".into(),
            issuer: "Banco".into(),
            card_last_four: None,
            period: "2026-08".into(),
            closing_date: "2026-08-28".into(),
            due_date: "2026-09-08".into(),
            currency: "ARS".into(),
            previous_balance: "0".into(),
            payments_amount: "0".into(),
            credits_amount: "0".into(),
            purchases_amount: "150".into(),
            fees_amount: "0".into(),
            interest_amount: "0".into(),
            taxes_amount: "0".into(),
            total_due: "150".into(),
            minimum_payment: None,
            status: "confirmed".into(),
            created_at: None,
            source_reference: Some("statement.pdf".into()),
            raw_extraction: Some("raw".into()),
            items: vec![],
        };
        for _ in 0..2 {
            let transaction = connection.transaction().expect("transaction");
            persist_card_reconciliation(
                &transaction,
                statement.source_reference.as_deref(),
                statement.raw_extraction.as_deref(),
                "statement-artifact",
                &reconciliation,
                "user-owner",
                "app",
            )
            .expect("persist reconciliation");
            transaction.commit().expect("commit reconciliation");
        }
        let occurrence: (Option<String>, String, String) = connection
            .query_row(
                "SELECT paid_amount,created_at,status FROM finance_service_occurrences
                 WHERE id='occurrence'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("persisted occurrence");
        assert_eq!(occurrence.0.as_deref(), Some("150.00"));
        assert_eq!(occurrence.1, "original");
        assert_eq!(occurrence.2, "accepted");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM finance_service_occurrence_versions
                     WHERE occurrence_id='occurrence'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("version count"),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT service_id FROM finance_transactions WHERE id='card-line'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .expect("transaction link")
                .as_deref(),
            Some("internet")
        );
        let mut invalid = reconciliation.clone();
        invalid.assignments[0].amount = "999.00".into();
        let transaction = connection.transaction().expect("rollback transaction");
        assert!(persist_card_reconciliation(
            &transaction,
            statement.source_reference.as_deref(),
            statement.raw_extraction.as_deref(),
            "statement-artifact",
            &invalid,
            "user-owner",
            "app",
        )
        .is_err());
        drop(transaction);
        assert_eq!(
            connection
                .query_row(
                    "SELECT paid_amount FROM finance_service_occurrences WHERE id='occurrence'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .expect("rollback keeps occurrence"),
            Some("150.00".into())
        );
    }

    // -----------------------------------------------------------------------
    // One expense per purchase: tickets, statements and installments
    // -----------------------------------------------------------------------

    fn finance_db() -> rusqlite::Connection {
        let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
        crate::database::migrate(&connection).expect("finance migrations");
        connection
            .execute_batch(
                "INSERT INTO finance_accounts(id,name,account_type,currency,opening_balance,active,created_at,updated_at) VALUES
                    ('card','Visa','credit_card','ARS','0',1,'now','now'),
                    ('bank','Banco','bank','ARS','0',1,'now','now');",
            )
            .expect("accounts");
        connection
    }

    fn owner_context() -> FinanceContext {
        FinanceContext {
            library_path: String::new(),
            android_directory_uri: None,
            actor_library_user_id: "user-owner".into(),
            source: "telegram".into(),
        }
    }

    fn ticket(id: &str, merchant: &str, date: &str, total: &str, product: &str) -> PurchaseRecord {
        PurchaseRecord {
            id: id.into(),
            account_id: "card".into(),
            category_id: Some("default-expense-food".into()),
            service_id: None,
            merchant_name: merchant.into(),
            observed_at: date.into(),
            currency: "ARS".into(),
            subtotal_amount: total.into(),
            discount_amount: "0".into(),
            tax_amount: "0".into(),
            total_amount: total.into(),
            status: "confirmed".into(),
            source_reference: Some(format!("telegram-photo:{id}")),
            raw_extraction: None,
            content_hash: None,
            items: vec![PurchaseItem {
                id: format!("{id}-item"),
                original_description: product.into(),
                normalized_description: None,
                quantity: "1".into(),
                unit_price: total.into(),
                discount_amount: "0".into(),
                line_total: total.into(),
                category_id: None,
            }],
        }
    }

    /// A paid statement; each line is (id, description, purchase date,
    /// amount, installment).
    fn paid_statement(
        id: &str,
        period: &str,
        due_date: &str,
        lines: &[(&str, &str, &str, &str, Option<(u16, u16)>)],
    ) -> CreditCardStatement {
        let total = lines
            .iter()
            .map(|line| parse_cents(line.3).expect("line amount"))
            .sum::<i128>();
        CreditCardStatement {
            id: id.into(),
            account_id: "card".into(),
            issuer: "Banco".into(),
            card_last_four: None,
            period: period.into(),
            closing_date: format!("{period}-28"),
            due_date: due_date.into(),
            currency: "ARS".into(),
            previous_balance: "0".into(),
            payments_amount: "0".into(),
            credits_amount: "0".into(),
            purchases_amount: format_cents(total),
            fees_amount: "0".into(),
            interest_amount: "0".into(),
            taxes_amount: "0".into(),
            total_due: format_cents(total),
            minimum_payment: None,
            status: "confirmed".into(),
            created_at: None,
            source_reference: Some(format!("{id}.pdf")),
            raw_extraction: None,
            items: lines
                .iter()
                .map(|(line_id, description, date, amount, installment)| CreditCardStatementItem {
                    id: (*line_id).into(),
                    purchase_date: (*date).into(),
                    description: (*description).into(),
                    amount: (*amount).into(),
                    currency: "ARS".into(),
                    item_type: "purchase".into(),
                    installment_number: installment.map(|value| value.0),
                    installment_count: installment.map(|value| value.1),
                    transaction_id: None,
                    category_id: None,
                })
                .collect(),
        }
    }

    /// Live expenses as (id, status, effective date).
    fn live_expenses(connection: &rusqlite::Connection) -> Vec<(String, String, String)> {
        connection
            .prepare(
                "SELECT id,status,effective_date FROM finance_transactions
                 WHERE deleted_at IS NULL AND transaction_type='expense' ORDER BY id",
            )
            .expect("expense query")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("expense rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("expenses")
    }

    fn line_transaction(connection: &rusqlite::Connection, item_id: &str) -> String {
        connection
            .query_row(
                "SELECT transaction_id FROM finance_credit_card_statement_items WHERE id=?1",
                [item_id],
                |row| row.get(0),
            )
            .expect("line expense")
    }

    #[test]
    fn a_card_ticket_waits_for_its_statement_and_counts_when_it_is_paid() {
        let mut connection = finance_db();
        let saved = store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context())
            .expect("ticket");
        assert!(saved.outcome.links.is_empty());
        assert_eq!(
            live_expenses(&connection),
            vec![("purchase:p1".into(), CARD_UNPAID.into(), "2026-08-12".into())]
        );

        let saved = store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "MERPAGO*COTO 4521", "2026-08-12", "1500", None)]),
            &owner_context(),
        )
        .expect("statement");

        assert_eq!(saved.outcome.links.len(), 1);
        assert_eq!(saved.created_transactions, 0);
        assert_eq!(
            live_expenses(&connection),
            vec![("purchase:p1".into(), "confirmed".into(), "2026-09-05".into())]
        );
        assert_eq!(line_transaction(&connection, "l1"), "purchase:p1");
    }

    #[test]
    fn a_ticket_sent_after_its_statement_joins_the_line_it_was_paid_with() {
        let mut connection = finance_db();
        store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "COTO CICSA 123", "2026-08-12", "1500", None)]),
            &owner_context(),
        )
        .expect("statement");
        let line = line_transaction(&connection, "l1");

        let saved = store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context())
            .expect("ticket");

        assert_eq!(saved.outcome.links.len(), 1);
        assert_eq!(live_expenses(&connection), vec![(line.clone(), "confirmed".into(), "2026-09-05".into())]);
        let (purchase_transaction, description, merchant): (String, String, Option<String>) = connection
            .query_row(
                "SELECT p.transaction_id,t.description,t.merchant_id FROM finance_purchases p
                 JOIN finance_transactions t ON t.id=p.transaction_id WHERE p.id='p1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("ticket link");
        assert_eq!(purchase_transaction, line);
        assert_eq!(description, "Compra en Coto");
        assert!(merchant.is_some());
    }

    #[test]
    fn two_possible_tickets_for_one_line_ask_which_one_it_was() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context()).expect("first ticket");
        store_purchase(&mut connection, &ticket("p2", "Coto", "2026-08-13", "1500", "Café"), &owner_context()).expect("second ticket");

        let saved = store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "COTO", "2026-08-12", "1500", None)]),
            &owner_context(),
        )
        .expect("statement");

        assert!(saved.outcome.links.is_empty());
        assert_eq!(saved.outcome.review_items.len(), 1);
        let item = &saved.outcome.review_items[0];
        assert_eq!(item.kind, "card-line-candidates");
        assert_eq!(item.options.len(), 3);

        crate::finance_matching::resolve_review_item(&connection, &item.id, "merge:purchase:p1", "user-owner", "app")
            .expect("resolution");
        let unpaid = live_expenses(&connection)
            .into_iter()
            .filter(|expense| expense.1 == CARD_UNPAID)
            .map(|expense| expense.0)
            .collect::<Vec<_>>();
        assert_eq!(unpaid, vec!["purchase:p2".to_string()]);
    }

    #[test]
    fn installment_lines_build_and_follow_their_plan() {
        let mut connection = finance_db();
        store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "FRAVEGA C.01/03", "2026-08-10", "333.33", Some((1, 3)))]),
            &owner_context(),
        )
        .expect("first statement");
        let plan = |connection: &rusqlite::Connection| -> Vec<(i64, String, String, Option<String>)> {
            connection
                .prepare("SELECT installment_number,due_date,status,transaction_id FROM finance_installments ORDER BY installment_number")
                .expect("installments")
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
                .expect("installment rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("plan")
        };
        let first = plan(&connection);
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].2, "confirmed");
        assert_eq!(first[0].3.as_deref(), Some(line_transaction(&connection, "l1").as_str()));
        assert_eq!((first[1].1.as_str(), first[1].2.as_str()), ("2026-10-05", "pending"));
        assert_eq!((first[2].1.as_str(), first[2].2.as_str()), ("2026-11-05", "pending"));

        let saved = store_credit_card_statement(
            &mut connection,
            &paid_statement("s2", "2026-09", "2026-10-05", &[("l2", "FRAVEGA C.02/03", "2026-08-10", "333.33", Some((2, 3)))]),
            &owner_context(),
        )
        .expect("second statement");

        assert_eq!(saved.outcome.links.len(), 1);
        let second = plan(&connection);
        assert_eq!(second.len(), 3);
        assert_eq!(second[1].2, "confirmed");
        assert_eq!(second[1].3.as_deref(), Some(line_transaction(&connection, "l2").as_str()));
        let plans: i64 = connection
            .query_row("SELECT COUNT(*) FROM finance_installment_plans", [], |row| row.get(0))
            .expect("plan count");
        assert_eq!(plans, 1);
    }

    #[test]
    fn a_ticket_bought_in_installments_joins_its_first_installment() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Frávega", "2026-08-10", "1000", "Heladera"), &owner_context()).expect("ticket");

        store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "FRAVEGA C.01/03", "2026-08-10", "333.33", Some((1, 3)))]),
            &owner_context(),
        )
        .expect("statement");

        let line = line_transaction(&connection, "l1");
        assert_eq!(live_expenses(&connection), vec![(line.clone(), "confirmed".into(), "2026-09-05".into())]);
        let purchase_transaction: String = connection
            .query_row("SELECT transaction_id FROM finance_purchases WHERE id='p1'", [], |row| row.get(0))
            .expect("ticket link");
        assert_eq!(purchase_transaction, line);
    }

    #[test]
    fn removing_a_statement_line_sends_its_ticket_back_to_wait() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context()).expect("ticket");
        let mut statement = paid_statement(
            "s1",
            "2026-08",
            "2026-09-05",
            &[("l1", "COTO", "2026-08-12", "1500", None), ("l2", "SPOTIFY", "2026-08-20", "300", None)],
        );
        store_credit_card_statement(&mut connection, &statement, &owner_context()).expect("statement");

        statement = paid_statement("s1", "2026-08", "2026-09-05", &[("l2", "SPOTIFY", "2026-08-20", "300", None)]);
        store_credit_card_statement(&mut connection, &statement, &owner_context()).expect("statement without the line");

        let expenses = live_expenses(&connection);
        assert!(expenses.contains(&("purchase:p1".into(), CARD_UNPAID.into(), "2026-08-12".into())));
        assert_eq!(expenses.len(), 2);
    }

    #[test]
    fn undoing_an_automatic_link_splits_the_ticket_from_the_line() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context()).expect("ticket");
        let saved = store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "COTO", "2026-08-12", "1500", None)]),
            &owner_context(),
        )
        .expect("statement");

        crate::finance_matching::unlink_line(&connection, &saved.outcome.links[0].id).expect("undo");

        let line = line_transaction(&connection, "l1");
        assert_ne!(line, "purchase:p1");
        let expenses = live_expenses(&connection);
        assert!(expenses.contains(&(line, "confirmed".into(), "2026-09-05".into())));
        assert!(expenses.contains(&("purchase:p1".into(), CARD_UNPAID.into(), "2026-08-12".into())));
    }

    #[test]
    fn deleting_a_statement_keeps_the_tickets_it_had_paid() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Yerba"), &owner_context()).expect("ticket");
        store_credit_card_statement(
            &mut connection,
            &paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "COTO", "2026-08-12", "1500", None), ("l2", "SPOTIFY", "2026-08-20", "300", None)]),
            &owner_context(),
        )
        .expect("statement");

        crate::finance_edits::delete_record(&connection, "card-statement", "s1").expect("delete statement");

        assert_eq!(
            live_expenses(&connection),
            vec![("purchase:p1".into(), CARD_UNPAID.into(), "2026-08-12".into())]
        );
        let statements: i64 = connection
            .query_row("SELECT COUNT(*) FROM finance_credit_card_statements", [], |row| row.get(0))
            .expect("statement count");
        assert_eq!(statements, 0);
    }

    #[test]
    fn a_similar_product_name_is_asked_about_and_merged_on_request() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p1", "Coto", "2026-08-12", "1500", "Leche La Serenísima 1L"), &owner_context())
            .expect("first ticket");
        let saved = store_purchase(&mut connection, &ticket("p2", "Coto", "2026-08-19", "1600", "LECHE SEREN 1LT"), &owner_context())
            .expect("second ticket");

        let item = saved
            .outcome
            .review_items
            .iter()
            .find(|item| item.kind == "similar-product")
            .expect("similar product question");
        crate::finance_matching::resolve_review_item(&connection, &item.id, "merge", "user-owner", "app").expect("merge");

        let products: i64 = connection
            .query_row("SELECT COUNT(*) FROM finance_products", [], |row| row.get(0))
            .expect("product count");
        let observations: i64 = connection
            .query_row("SELECT COUNT(DISTINCT product_id) FROM finance_price_observations", [], |row| row.get(0))
            .expect("observation products");
        assert_eq!((products, observations), (1, 1));

        // The abbreviation is now an alias: a third ticket asks nothing.
        let saved = store_purchase(&mut connection, &ticket("p3", "Coto", "2026-08-26", "1700", "LECHE SEREN 1LT"), &owner_context())
            .expect("third ticket");
        assert!(saved.outcome.review_items.is_empty());
    }

    #[test]
    fn card_fees_are_card_charges_and_purchases_take_the_merchant_category() {
        let mut connection = finance_db();
        store_purchase(&mut connection, &ticket("p0", "Coto", "2026-07-01", "100", "Pan"), &owner_context()).expect("known merchant");
        let mut statement = paid_statement("s1", "2026-08", "2026-09-05", &[("l1", "COTO", "2026-08-20", "900", None)]);
        statement.items.push(CreditCardStatementItem {
            id: "fee".into(),
            purchase_date: "2026-08-28".into(),
            description: "Comisión mantenimiento".into(),
            amount: "50".into(),
            currency: "ARS".into(),
            item_type: "fee".into(),
            installment_number: None,
            installment_count: None,
            transaction_id: None,
            category_id: None,
        });
        statement.fees_amount = "50".into();
        statement.total_due = "950".into();
        store_credit_card_statement(&mut connection, &statement, &owner_context()).expect("statement");

        let category = |item: &str| -> Option<String> {
            connection
                .query_row(
                    "SELECT t.category_id FROM finance_credit_card_statement_items i
                     JOIN finance_transactions t ON t.id=i.transaction_id WHERE i.id=?1",
                    [item],
                    |row| row.get(0),
                )
                .expect("line category")
        };
        assert_eq!(category("fee").as_deref(), Some(CARD_CHARGES_CATEGORY));
        assert_eq!(category("l1").as_deref(), Some("default-expense-food"));
    }
}
