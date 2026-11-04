//! Normalización y validación de las altas financieras pedidas por el agente.
//!
//! Los modelos envían importes en formatos locales, nombres en lugar de IDs y
//! omiten campos que el dominio exige (`status`, IDs de línea). Este módulo
//! convierte esos argumentos en registros de dominio válidos o en un error
//! estructurado que indica exactamente qué campos corregir. No persiste nada:
//! la confirmación y el guardado quedan en el runtime y en `finance*`.

use serde_json::{json, Value};

use crate::finance::{FinanceAccount, FinanceCategory, FinanceService, FinanceTransaction};
use crate::finance_records::{
    CreditCardStatement, CreditCardStatementItem, PurchaseItem, PurchaseRecord, SalaryConcept,
    SalaryReceipt,
};

const MAX_PURCHASE_ITEMS: usize = 100;
const MAX_SALARY_CONCEPTS: usize = 200;
const MAX_STATEMENT_ITEMS: usize = 500;
const MAX_RAW_TEXT_CHARS: usize = 20_000;
const MAX_NAME_CHARS: usize = 200;
const MAX_CATEGORY_NAME_CHARS: usize = 80;
const MAX_CATEGORY_DESCRIPTION_CHARS: usize = 500;
const MAX_SERVICE_NAME_CHARS: usize = 160;
const CURRENCIES: [&str; 2] = ["ARS", "USD"];
const STATEMENT_ITEM_TYPES: [&str; 6] = ["purchase", "fee", "interest", "tax", "payment", "credit"];

/// Error recuperable para el modelo: nombra los campos y cómo corregirlos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentInputError {
    pub error: &'static str,
    pub invalid_fields: Vec<String>,
    pub instruction: String,
}

/// What a field asks for, so the model knows what to fix or ask.
const FIELD_HINTS: [(&str, &str); 7] = [
    ("name", "el nombre (por ejemplo «Internet Fibertel»)"),
    ("expectedAmount", "el importe esperado por período, un número positivo como 15000 o 15.000,50"),
    ("amount", "el importe, un número positivo como 15000 o 15.000,50"),
    ("currency", "la moneda: ARS o USD"),
    ("categoryId", "el ID o el nombre exacto de una categoría existente (list_finance_categories)"),
    ("accountId", "el ID o el nombre exacto de una cuenta (list_finance_accounts)"),
    ("effectiveDate", "la fecha, AAAA-MM-DD"),
];

impl AgentInputError {
    fn fields(error: &'static str, invalid_fields: Vec<&str>) -> Self {
        let described = invalid_fields
            .iter()
            .map(|field| match FIELD_HINTS.iter().find(|(name, _)| name == field) {
                Some((_, hint)) => format!("{field} ({hint})"),
                None => field.to_string(),
            })
            .collect::<Vec<_>>();
        let invalid_fields = invalid_fields.into_iter().map(str::to_string).collect::<Vec<_>>();
        let instruction = format!(
            "Faltan o no son válidos estos campos: {}. Si la persona no te dio esos datos, preguntáselos antes de reintentar; no los inventes.",
            described.join("; ")
        );
        Self {
            error,
            invalid_fields,
            instruction,
        }
    }

    fn with_instruction(error: &'static str, field: &str, instruction: &str) -> Self {
        Self {
            error,
            invalid_fields: vec![field.to_string()],
            instruction: instruction.to_string(),
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        json!({
            "ok": false,
            "changed": false,
            "error": self.error,
            "invalidFields": self.invalid_fields,
            "instruction": self.instruction,
            "requiresClarification": true,
        })
    }
}

/// Resultado de un alta que puede coincidir con una entidad existente.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CreateOrExisting<T, E> {
    Create(T),
    Existing(E),
}

/// Acepta formatos canónicos, numéricos y locales ARS/USD (`1.234,56`,
/// `$ 1,234.56`, `1234`). Rechaza negativos, notación científica y más
/// decimales de los permitidos. Devuelve el decimal canónico sin ceros finales.
pub(crate) fn normalize_decimal(value: &Value, max_fraction_digits: usize) -> Option<String> {
    if max_fraction_digits > 8 {
        return None;
    }
    let raw = match value {
        Value::Number(number) => {
            let text = number.to_string();
            if text.starts_with('-') || text.contains(['e', 'E']) {
                return None;
            }
            let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
            if fraction.len() > max_fraction_digits {
                return None;
            }
            return Some(canonical_decimal(whole, fraction));
        }
        Value::String(text) => text.trim().to_string(),
        _ => return None,
    };
    if raw.is_empty() {
        return None;
    }
    let mut compact = raw.clone();
    for symbol in ["US$", "u$s", "U$S", "ARS", "ars", "USD", "usd", "$"] {
        compact = compact.replace(symbol, "");
    }
    let compact = compact.split_whitespace().collect::<String>();
    if compact.is_empty()
        || compact.starts_with('-')
        || compact.chars().any(|c| !(c.is_ascii_digit() || c == '.' || c == ','))
    {
        return None;
    }
    let comma = compact.rfind(',');
    let dot = compact.rfind('.');
    let last_separator = comma.max(dot);
    let (mut integer, mut fraction) = (compact.as_str(), "");
    if let Some(index) = last_separator {
        let separator = compact.as_bytes()[index] as char;
        let trailing = compact.len() - index - 1;
        let separator_count = compact.chars().filter(|c| *c == separator).count();
        let has_both = comma.is_some() && dot.is_some();
        if trailing > 0 && trailing <= max_fraction_digits && (has_both || separator_count == 1) {
            integer = &compact[..index];
            fraction = &compact[index + 1..];
        }
    }
    let integer = integer.replace(['.', ','], "");
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    let integer = if integer.is_empty() { "0".to_string() } else { integer };
    if !integer.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(canonical_decimal(&integer, fraction))
}

fn canonical_decimal(whole: &str, fraction: &str) -> String {
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    }
}

const MAX_SPLITS: usize = 10;

/// Los argumentos de cada movimiento de un alta dividida en categorías
/// (`splits`: importe, categoría y descripción opcional de cada parte), o
/// `None` si no se divide. Las partes suman exactamente `amount`; cada una
/// conserva el resto de los datos del alta.
pub(crate) fn split_parts(args: &Value) -> Result<Option<Vec<Value>>, AgentInputError> {
    let Some(splits) = args.get("splits").filter(|splits| !splits.is_null()) else {
        return Ok(None);
    };
    let splits = splits.as_array().filter(|splits| (2..=MAX_SPLITS).contains(&splits.len())).ok_or_else(|| {
        AgentInputError::with_instruction("invalid-finance-splits", "splits", "Dividí en entre 2 y 10 partes, cada una con amount y categoryId.")
    })?;
    let total = args.get("amount").and_then(|value| normalize_decimal(value, 2));
    let mut parts = Vec::new();
    let mut amounts = Vec::new();
    for split in splits {
        let amount = split.get("amount").and_then(|value| normalize_decimal(value, 2));
        let category = text(split, "categoryId");
        let (Some(amount), false) = (amount, category.is_empty()) else {
            return Err(AgentInputError::with_instruction("invalid-finance-splits", "splits", "Cada parte necesita amount (positivo) y categoryId."));
        };
        let mut part = args.clone();
        if let Some(object) = part.as_object_mut() {
            object.remove("splits");
            object.insert("amount".into(), json!(amount));
            object.insert("categoryId".into(), json!(category));
            if let Some(description) = optional_text(split, "description") {
                object.insert("description".into(), json!(description));
            }
        }
        amounts.push(amount);
        parts.push(part);
    }
    let sum = sum_amounts(amounts.iter().map(String::as_str));
    match (total, sum) {
        (Some(total), Some(sum)) if crate::finance::parse_cents(&total).ok() == crate::finance::parse_cents(&sum).ok() => Ok(Some(parts)),
        (Some(total), Some(sum)) => Err(AgentInputError::with_instruction(
            "finance-splits-do-not-add-up",
            "splits",
            &format!("Las partes suman {sum} y el total (amount) es {total}: tienen que sumar exactamente el total."),
        )),
        _ => Err(AgentInputError::fields("invalid-finance-splits", vec!["amount"])),
    }
}

/// Suma importes con dos decimales sin pérdida de precisión.
pub(crate) fn sum_amounts<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut total_cents = 0_i128;
    for value in values {
        total_cents = total_cents.checked_add(crate::finance::parse_cents(value).ok()?)?;
    }
    let formatted = crate::finance::format_cents(total_cents);
    Some(canonical_decimal(
        formatted.split('.').next().unwrap_or("0"),
        formatted.split('.').nth(1).unwrap_or(""),
    ))
}

fn text(args: &Value, name: &str) -> String {
    args.get(name)
        .and_then(Value::as_str)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn optional_text(args: &Value, name: &str) -> Option<String> {
    Some(text(args, name)).filter(|value| !value.is_empty())
}

fn raw_text(args: &Value, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(|value| value.chars().take(MAX_RAW_TEXT_CHARS).collect())
}

fn currency(args: &Value, name: &str) -> Option<String> {
    let value = text(args, name).to_ascii_uppercase();
    CURRENCIES.contains(&value.as_str()).then_some(value)
}

fn period(args: &Value, name: &str) -> Option<String> {
    let value = text(args, name);
    crate::finance::valid_service_period(&value).then_some(value)
}

fn iso_date(args: &Value, name: &str) -> Option<String> {
    let value = text(args, name);
    crate::finance::valid_iso_date(&value).then_some(value)
}

fn fold_name(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    value
        .nfd()
        .filter(|character| !unicode_normalization::char::is_combining_mark(*character))
        .collect::<String>()
        .trim()
        .to_lowercase()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Resuelve una cuenta activa por ID exacto o por nombre sin acentos.
pub(crate) fn resolve_account<'a>(
    accounts: &'a [FinanceAccount],
    value: &str,
) -> Option<&'a FinanceAccount> {
    let folded = fold_name(value);
    accounts
        .iter()
        .find(|account| account.active && (account.id == value || fold_name(&account.name) == folded))
}

/// Resuelve una categoría activa del tipo pedido por ID o nombre.
pub(crate) fn resolve_category<'a>(
    categories: &'a [FinanceCategory],
    value: &str,
    kind: &str,
) -> Option<&'a FinanceCategory> {
    let folded = fold_name(value);
    categories.iter().find(|category| {
        category.active
            && category.kind == kind
            && (category.id == value || fold_name(&category.name) == folded)
    })
}

pub(crate) fn build_category(
    args: &Value,
    categories: &[FinanceCategory],
) -> Result<CreateOrExisting<FinanceCategory, FinanceCategory>, AgentInputError> {
    let name = text(args, "name");
    let kind = text(args, "kind");
    let description = text(args, "description");
    let mut invalid = Vec::new();
    if name.is_empty() || name.chars().count() > MAX_CATEGORY_NAME_CHARS {
        invalid.push("name");
    }
    if kind != "income" && kind != "expense" {
        invalid.push("kind");
    }
    if description.chars().count() > MAX_CATEGORY_DESCRIPTION_CHARS {
        invalid.push("description");
    }
    if !invalid.is_empty() {
        return Err(AgentInputError::fields("invalid-finance-category", invalid));
    }
    if let Some(existing) = resolve_category(categories, &name, &kind) {
        return Ok(CreateOrExisting::Existing(existing.clone()));
    }
    Ok(CreateOrExisting::Create(FinanceCategory {
        id: new_id(),
        name,
        kind,
        active: true,
        parent_id: None,
        description: Some(description).filter(|value| !value.is_empty()),
    }))
}

pub(crate) fn build_service(
    args: &Value,
    categories: &[FinanceCategory],
    services: &[FinanceService],
) -> Result<CreateOrExisting<FinanceService, FinanceService>, AgentInputError> {
    let name = text(args, "name");
    let expected_amount = args
        .get("expectedAmount")
        .and_then(|value| normalize_decimal(value, 2));
    let currency_value = currency(args, "currency");
    let category_value = text(args, "categoryId");
    let mut invalid = Vec::new();
    if name.is_empty() || name.chars().count() > MAX_SERVICE_NAME_CHARS {
        invalid.push("name");
    }
    if expected_amount.is_none() {
        invalid.push("expectedAmount");
    }
    if currency_value.is_none() {
        invalid.push("currency");
    }
    if category_value.is_empty() {
        invalid.push("categoryId");
    }
    if !invalid.is_empty() {
        return Err(AgentInputError::fields("invalid-finance-service", invalid));
    }
    let category = resolve_category(categories, &category_value, "expense").ok_or_else(|| {
        AgentInputError::with_instruction(
            "finance-service-category-not-found",
            "categoryId",
            "Usa el ID o el nombre exacto de una categoría de gastos activa.",
        )
    })?;
    let provider = optional_text(args, "provider");
    let folded_name = fold_name(&name);
    let folded_provider = provider.as_deref().map(fold_name);
    if let Some(existing) = services.iter().find(|service| {
        service.active
            && fold_name(&service.name) == folded_name
            && (folded_provider.is_none()
                || service.provider.as_deref().map(fold_name) == folded_provider)
    }) {
        return Ok(CreateOrExisting::Existing(existing.clone()));
    }
    let due_day = args
        .get("dueDay")
        .and_then(Value::as_i64)
        .filter(|day| (1..=31).contains(day));
    Ok(CreateOrExisting::Create(FinanceService {
        id: new_id(),
        name,
        category_id: category.id.clone(),
        currency: currency_value.unwrap_or_default(),
        expected_amount: expected_amount.unwrap_or_default(),
        due_day,
        default_account_id: optional_text(args, "defaultAccountId"),
        provider,
        modality: if text(args, "modality") == "variable" {
            "variable".to_string()
        } else {
            "fixed".to_string()
        },
        active: true,
        created_at: None,
        updated_at: None,
    }))
}

pub(crate) fn build_purchase(
    args: &Value,
    accounts: &[FinanceAccount],
    categories: &[FinanceCategory],
    services: &[FinanceService],
    default_source_reference: &str,
    today: &str,
) -> Result<PurchaseRecord, AgentInputError> {
    let account_value = text(args, "accountId");
    let merchant_name = text(args, "merchantName");
    let currency_value = currency(args, "currency");
    let observed_at = {
        let value = text(args, "observedAt");
        if value.len() >= 10 && crate::finance::valid_iso_date(&value[..10]) {
            value
        } else {
            format!("{today}T12:00:00Z")
        }
    };
    let decimal = |name: &str| args.get(name).and_then(|value| normalize_decimal(value, 2));
    let discount_amount = decimal("discountAmount").unwrap_or_else(|| "0".to_string());
    let tax_amount = decimal("taxAmount").unwrap_or_else(|| "0".to_string());
    let total_amount = decimal("totalAmount");
    // A product removed at the till is marked `voided`: it is neither added
    // nor saved, and its negative line is not sent.
    let raw_items = args
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_PURCHASE_ITEMS)
                .filter(|item| item.get("voided").and_then(Value::as_bool) != Some(true))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let item_problems = raw_items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| purchase_item_problem(index, item))
        .collect::<Vec<_>>();
    let items = raw_items
        .iter()
        .filter_map(|item| {
            let original_description = text(item, "originalDescription");
            let quantity = item.get("quantity").and_then(|value| normalize_decimal(value, 6))?;
            let unit_price = item.get("unitPrice").and_then(|value| normalize_decimal(value, 2))?;
            let line_total = item.get("lineTotal").and_then(|value| normalize_decimal(value, 2))?;
            if original_description.is_empty() {
                return None;
            }
            Some(PurchaseItem {
                id: new_id(),
                original_description,
                normalized_description: optional_text(item, "normalizedDescription"),
                quantity,
                unit_price,
                discount_amount: item
                    .get("discountAmount")
                    .and_then(|value| normalize_decimal(value, 2))
                    .unwrap_or_else(|| "0".to_string()),
                line_total,
                category_id: None,
            })
        })
        .collect::<Vec<_>>();
    let subtotal_amount = sum_amounts(items.iter().map(|item| item.line_total.as_str()));
    let mut invalid = Vec::new();
    if account_value.is_empty() {
        invalid.push("accountId");
    }
    if merchant_name.is_empty() || merchant_name.chars().count() > MAX_NAME_CHARS {
        invalid.push("merchantName");
    }
    if currency_value.is_none() {
        invalid.push("currency");
    }
    if subtotal_amount.is_none() {
        invalid.push("subtotalAmount");
    }
    if total_amount.is_none() {
        invalid.push("totalAmount");
    }
    if raw_items.is_empty() {
        invalid.push("items");
    }
    if items.len() != raw_items.len() {
        invalid.push("itemAmounts");
    }
    if !invalid.is_empty() {
        let mut error = AgentInputError::fields("invalid-finance-purchase", invalid);
        if !item_problems.is_empty() {
            error.instruction.push_str(&format!(" Líneas a corregir: {}.", item_problems.join("; ")));
        }
        return Err(error);
    }
    let currency_value = currency_value.unwrap_or_default();
    let account = resolve_account(accounts, &account_value)
        .filter(|account| account.currency == currency_value)
        .ok_or_else(|| {
            AgentInputError::with_instruction(
                "finance-purchase-account-invalid",
                "accountId",
                "Usa el ID exacto de una cuenta listada con la misma moneda del ticket.",
            )
        })?;
    let category = resolve_category(categories, &text(args, "categoryId"), "expense");
    let service_value = text(args, "serviceId");
    let service = if service_value.is_empty() {
        None
    } else {
        let service = services
            .iter()
            .find(|service| service.active && service.id == service_value)
            .ok_or_else(|| {
                AgentInputError::with_instruction(
                    "finance-service-not-found",
                    "serviceId",
                    "Usa el ID exacto de un servicio activo o omite serviceId.",
                )
            })?;
        if service.currency != currency_value {
            return Err(AgentInputError::with_instruction(
                "finance-service-expense-currency-mismatch",
                "serviceId",
                "El servicio y el ticket deben usar la misma moneda.",
            ));
        }
        Some(service)
    };
    let category_id = category.map(|category| category.id.clone());
    Ok(PurchaseRecord {
        id: new_id(),
        account_id: account.id.clone(),
        category_id: category_id.clone(),
        service_id: service.map(|service| service.id.clone()),
        merchant_name,
        observed_at,
        currency: currency_value,
        subtotal_amount: subtotal_amount.unwrap_or_default(),
        discount_amount,
        tax_amount,
        total_amount: total_amount.unwrap_or_default(),
        status: "confirmed".to_string(),
        source_reference: Some(
            optional_text(args, "sourceReference")
                .unwrap_or_else(|| default_source_reference.to_string()),
        ),
        raw_extraction: raw_text(args, "rawExtraction"),
        content_hash: None,
        items: items
            .into_iter()
            .map(|item| PurchaseItem {
                category_id: category_id.clone(),
                ..item
            })
            .collect(),
    })
}

/// What is wrong with a line of a ticket, so the agent fixes that line
/// instead of guessing what `itemAmounts` means.
fn purchase_item_problem(index: usize, item: &Value) -> Option<String> {
    let description = text(item, "originalDescription");
    let mut problems = Vec::new();
    if description.is_empty() {
        problems.push("falta originalDescription".to_string());
    }
    for (field, digits) in [("quantity", 6), ("unitPrice", 2), ("lineTotal", 2)] {
        match item.get(field) {
            None | Some(Value::Null) => problems.push(format!("falta {field}")),
            Some(value) if normalize_decimal(value, digits).is_some() => {}
            Some(value) => {
                let shown = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
                problems.push(if shown.trim().starts_with('-') {
                    format!("{field} es negativo («{shown}»): si es la anulación de un producto, marcá ese producto con voided: true y no mandes la línea negativa")
                } else {
                    format!("{field} «{shown}» no es un número positivo con hasta {digits} decimales (por ejemplo «1234.56» o «1.234,56»)")
                });
            }
        }
    }
    if problems.is_empty() {
        return None;
    }
    let name = if description.is_empty() { String::new() } else { format!(" («{description}»)") };
    Some(format!("items[{index}]{name}: {}", problems.join(", ")))
}

pub(crate) fn build_salary(
    args: &Value,
    accounts: &[FinanceAccount],
    default_source_reference: &str,
) -> Result<SalaryReceipt, AgentInputError> {
    let account_value = text(args, "accountId");
    let period_value = period(args, "period");
    let payment_date = iso_date(args, "paymentDate");
    let employer = text(args, "employer");
    let decimal = |name: &str| args.get(name).and_then(|value| normalize_decimal(value, 2));
    let (gross, deductions, net) = (
        decimal("grossAmount"),
        decimal("deductionsTotal"),
        decimal("netAmount"),
    );
    let currency_value = currency(args, "currency");
    let raw_concepts = args
        .get("concepts")
        .and_then(Value::as_array)
        .map(|items| items.iter().take(MAX_SALARY_CONCEPTS).collect::<Vec<_>>())
        .unwrap_or_default();
    let concepts = raw_concepts
        .iter()
        .filter_map(|concept| {
            let name = text(concept, "name");
            let concept_type = text(concept, "conceptType");
            let amount = concept.get("amount").and_then(|value| normalize_decimal(value, 2))?;
            (!name.is_empty() && (concept_type == "earning" || concept_type == "deduction")).then(
                || SalaryConcept {
                    id: new_id(),
                    name,
                    concept_type,
                    amount,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut invalid = Vec::new();
    if account_value.is_empty() {
        invalid.push("accountId");
    }
    if period_value.is_none() {
        invalid.push("period");
    }
    if payment_date.is_none() {
        invalid.push("paymentDate");
    }
    if employer.is_empty() || employer.chars().count() > MAX_NAME_CHARS {
        invalid.push("employer");
    }
    if gross.is_none() {
        invalid.push("grossAmount");
    }
    if deductions.is_none() {
        invalid.push("deductionsTotal");
    }
    if net.is_none() {
        invalid.push("netAmount");
    }
    if currency_value.is_none() {
        invalid.push("currency");
    }
    if concepts.len() != raw_concepts.len() {
        invalid.push("concepts");
    }
    if !invalid.is_empty() {
        return Err(AgentInputError::fields("invalid-finance-salary", invalid));
    }
    let currency_value = currency_value.unwrap_or_default();
    let account = resolve_account(accounts, &account_value)
        .filter(|account| account.currency == currency_value)
        .ok_or_else(|| {
            AgentInputError::with_instruction(
                "finance-salary-account-invalid",
                "accountId",
                "Usa el ID exacto de una cuenta listada con la misma moneda del recibo.",
            )
        })?;
    let source_reference = optional_text(args, "sourceReference")
        .unwrap_or_else(|| default_source_reference.to_string());
    let signed_document = args.get("signedDocument").and_then(Value::as_bool) == Some(true);
    if signed_document && !source_reference.to_lowercase().ends_with(".pdf") {
        return Err(AgentInputError::with_instruction(
            "finance-salary-signed-pdf-required",
            "signedDocument",
            "Usa signedDocument=true solamente cuando la evidencia original sea un PDF firmado.",
        ));
    }
    Ok(SalaryReceipt {
        id: new_id(),
        period: period_value.unwrap_or_default(),
        payment_date: payment_date.unwrap_or_default(),
        employer,
        gross_amount: gross.unwrap_or_default(),
        deductions_total: deductions.unwrap_or_default(),
        net_amount: net.unwrap_or_default(),
        currency: currency_value,
        account_id: account.id.clone(),
        status: "confirmed".to_string(),
        signed_document,
        created_at: None,
        source_reference: Some(source_reference),
        raw_extraction: raw_text(args, "rawExtraction"),
        concepts,
    })
}

pub(crate) fn build_credit_card_statement(
    args: &Value,
    accounts: &[FinanceAccount],
    default_source_reference: &str,
) -> Result<CreditCardStatement, AgentInputError> {
    const AMOUNT_FIELDS: [&str; 8] = [
        "previousBalance",
        "paymentsAmount",
        "creditsAmount",
        "purchasesAmount",
        "feesAmount",
        "interestAmount",
        "taxesAmount",
        "totalDue",
    ];
    // Aggregated amounts without line detail become one synthetic line so the
    // statement balances; the item type mirrors the aggregate field.
    const AGGREGATES: [(&str, &str, &str); 6] = [
        ("paymentsAmount", "payment", "Pagos informados sin desglose"),
        ("creditsAmount", "credit", "Créditos informados sin desglose"),
        ("purchasesAmount", "purchase", "Consumos informados sin desglose"),
        ("feesAmount", "fee", "Cargos informados sin desglose"),
        ("interestAmount", "interest", "Intereses informados sin desglose"),
        ("taxesAmount", "tax", "Impuestos informados sin desglose"),
    ];
    let account_value = text(args, "accountId");
    let issuer = text(args, "issuer");
    let card_last_four = Some(text(args, "cardLastFour"))
        .filter(|value| value.len() == 4 && value.chars().all(|c| c.is_ascii_digit()));
    let period_value = period(args, "period");
    let closing_date = iso_date(args, "closingDate");
    let due_date = iso_date(args, "dueDate");
    let currency_value = currency(args, "currency");
    let amount = |name: &str| args.get(name).and_then(|value| normalize_decimal(value, 2));
    let amounts = AMOUNT_FIELDS
        .iter()
        .map(|field| (*field, amount(field)))
        .collect::<Vec<_>>();
    let minimum_payment_raw = args.get("minimumPayment").filter(|value| {
        !value.is_null() && value.as_str().map(str::trim) != Some("")
    });
    let minimum_payment = minimum_payment_raw.and_then(|value| normalize_decimal(value, 2));
    let raw_items = args
        .get("items")
        .and_then(Value::as_array)
        .map(|items| items.iter().take(MAX_STATEMENT_ITEMS).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut items = raw_items
        .iter()
        .filter_map(|item| {
            let purchase_date = iso_date(item, "purchaseDate")?;
            let description = text(item, "description");
            let amount = item.get("amount").and_then(|value| normalize_decimal(value, 2))?;
            let item_currency = currency(item, "currency").or_else(|| currency_value.clone())?;
            let item_type = text(item, "itemType");
            if description.is_empty()
                || !STATEMENT_ITEM_TYPES.contains(&item_type.as_str())
                || Some(&item_currency) != currency_value.as_ref()
            {
                return None;
            }
            let installment = |name: &str| {
                item.get(name)
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
            };
            Some(CreditCardStatementItem {
                id: new_id(),
                purchase_date,
                description,
                amount,
                currency: item_currency,
                item_type,
                installment_number: installment("installmentNumber"),
                installment_count: installment("installmentCount"),
                transaction_id: None,
                category_id: optional_text(item, "categoryId"),
            })
        })
        .collect::<Vec<_>>();
    let parsed_item_count = items.len();
    let mut invalid = Vec::new();
    if account_value.is_empty() {
        invalid.push("accountId");
    }
    if issuer.is_empty() || issuer.chars().count() > MAX_NAME_CHARS {
        invalid.push("issuer");
    }
    if period_value.is_none() {
        invalid.push("period");
    }
    if closing_date.is_none() {
        invalid.push("closingDate");
    }
    if due_date.is_none() {
        invalid.push("dueDate");
    }
    if currency_value.is_none() {
        invalid.push("currency");
    }
    invalid.extend(
        amounts
            .iter()
            .filter(|(_, value)| value.is_none())
            .map(|(field, _)| *field),
    );
    if minimum_payment_raw.is_some() && minimum_payment.is_none() {
        invalid.push("minimumPayment");
    }
    if raw_items.is_empty() || parsed_item_count == 0 {
        invalid.push("items");
    }
    if !invalid.is_empty() {
        return Err(AgentInputError::fields(
            "invalid-finance-credit-card-statement",
            invalid,
        ));
    }
    let value_of = |name: &str| {
        amounts
            .iter()
            .find(|(field, _)| *field == name)
            .and_then(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let currency_value = currency_value.unwrap_or_default();
    let closing_date = closing_date.unwrap_or_default();
    for (field, item_type, description) in AGGREGATES {
        let expected = value_of(field);
        if expected == "0" || items.iter().any(|item| item.item_type == item_type) {
            continue;
        }
        items.push(CreditCardStatementItem {
            id: new_id(),
            purchase_date: closing_date.clone(),
            description: description.to_string(),
            amount: expected,
            currency: currency_value.clone(),
            item_type: item_type.to_string(),
            installment_number: None,
            installment_count: None,
            transaction_id: None,
            category_id: None,
        });
    }
    let account = resolve_account(accounts, &account_value)
        .filter(|account| account.account_type == "credit_card")
        .ok_or_else(|| {
            AgentInputError::with_instruction(
                "finance-credit-card-account-invalid",
                "accountId",
                "Usa el ID exacto de una cuenta activa de tipo credit_card; la tarjeta puede tener líneas ARS y USD.",
            )
        })?;
    Ok(CreditCardStatement {
        id: new_id(),
        account_id: account.id.clone(),
        issuer,
        card_last_four,
        period: period_value.unwrap_or_default(),
        closing_date,
        due_date: due_date.unwrap_or_default(),
        currency: currency_value,
        previous_balance: value_of("previousBalance"),
        payments_amount: value_of("paymentsAmount"),
        credits_amount: value_of("creditsAmount"),
        purchases_amount: value_of("purchasesAmount"),
        fees_amount: value_of("feesAmount"),
        interest_amount: value_of("interestAmount"),
        taxes_amount: value_of("taxesAmount"),
        total_due: value_of("totalDue"),
        minimum_payment,
        status: "confirmed".to_string(),
        created_at: None,
        source_reference: Some(
            optional_text(args, "sourceReference")
                .unwrap_or_else(|| default_source_reference.to_string()),
        ),
        raw_extraction: raw_text(args, "rawExtraction"),
        items,
    })
}

/// Movimiento pedido por el agente, con cuentas, categoría y servicio
/// resueltos, y el servicio mensual que su pago debe marcar.
#[derive(Debug, Clone)]
pub(crate) struct TransactionDraft {
    pub transaction: FinanceTransaction,
    pub service: Option<FinanceService>,
}

/// Valida un alta de movimiento: importe local, fecha (hoy si falta),
/// cuenta y destino por ID o nombre, moneda de la cuenta, categoría del tipo
/// correcto y servicio de gasto en la misma moneda. `require_expense_category`
/// exige categoría para gastos (Telegram no puede mostrar un selector).
pub(crate) fn build_transaction(
    args: &Value,
    accounts: &[FinanceAccount],
    categories: &[FinanceCategory],
    services: &[FinanceService],
    today: &str,
    require_expense_category: bool,
) -> Result<TransactionDraft, AgentInputError> {
    let transaction_type = text(args, "transactionType");
    let raw_amount = args.get("amount").cloned().unwrap_or(Value::Null);
    let negative = raw_amount.as_str().is_some_and(|value| value.trim_start().starts_with('-'));
    let amount = match (&raw_amount, negative) {
        (Value::String(value), true) => normalize_decimal(&Value::String(value.trim_start()[1..].to_string()), 2)
            .map(|value| format!("-{value}")),
        _ => normalize_decimal(&raw_amount, 2),
    };
    let effective_date = match text(args, "effectiveDate") {
        value if value.is_empty() => Some(today.to_string()),
        value => crate::finance::valid_iso_date(&value).then_some(value),
    };
    let account_value = text(args, "accountId");
    let description = text(args, "description");
    let mut invalid = Vec::new();
    if !matches!(transaction_type.as_str(), "income" | "expense" | "transfer" | "adjustment") {
        invalid.push("transactionType");
    }
    if amount.is_none() || (negative && transaction_type != "adjustment") {
        invalid.push("amount");
    }
    if effective_date.is_none() {
        invalid.push("effectiveDate");
    }
    if account_value.is_empty() {
        invalid.push("accountId");
    }
    if description.is_empty() || description.chars().count() > 1_000 {
        invalid.push("description");
    }
    if !invalid.is_empty() {
        return Err(AgentInputError::fields("invalid-finance-transaction", invalid));
    }
    let account = resolve_account(accounts, &account_value).ok_or_else(|| {
        AgentInputError::with_instruction(
            "finance-account-not-found",
            "accountId",
            "Usa el ID o el nombre exacto de una cuenta activa (list_finance_accounts).",
        )
    })?;
    let currency_value = match text(args, "currency") {
        value if value.is_empty() => account.currency.clone(),
        value => value.to_ascii_uppercase(),
    };
    if currency_value != account.currency {
        return Err(AgentInputError::with_instruction(
            "finance-account-currency-mismatch",
            "currency",
            "La moneda debe ser la de la cuenta; nunca conviertas monedas.",
        ));
    }
    let destination_value = text(args, "destinationAccountId");
    let destination = if destination_value.is_empty() {
        None
    } else {
        Some(resolve_account(accounts, &destination_value).ok_or_else(|| {
            AgentInputError::with_instruction(
                "invalid-finance-transfer-account",
                "destinationAccountId",
                "Usa el ID o el nombre exacto de la cuenta que recibe el importe.",
            )
        })?)
    };
    let valid_transfer = match (transaction_type.as_str(), destination) {
        ("transfer", None) => false,
        (_, Some(destination)) => destination.currency == currency_value && destination.id != account.id,
        _ => true,
    };
    if !valid_transfer {
        return Err(AgentInputError::with_instruction(
            "invalid-finance-transfer-account",
            "destinationAccountId",
            "Una transferencia necesita otra cuenta activa de la misma moneda.",
        ));
    }
    let category_value = text(args, "categoryId");
    let category = if category_value.is_empty() {
        None
    } else {
        let kind = if transaction_type == "income" { "income" } else { "expense" };
        let found = if matches!(transaction_type.as_str(), "income" | "expense") {
            resolve_category(categories, &category_value, kind)
        } else {
            resolve_category(categories, &category_value, "expense")
                .or_else(|| resolve_category(categories, &category_value, "income"))
        };
        Some(found.ok_or_else(|| {
            AgentInputError::with_instruction(
                "finance-category-not-found",
                "categoryId",
                "Usa el ID o el nombre exacto de una categoría activa del mismo tipo que el movimiento.",
            )
        })?)
    };
    if transaction_type == "expense" && category.is_none() && require_expense_category {
        return Err(AgentInputError::with_instruction(
            "finance-expense-category-required",
            "categoryId",
            "Pedí la categoría del gasto antes de registrarlo.",
        ));
    }
    let service_value = text(args, "serviceId");
    let service = if service_value.is_empty() {
        None
    } else {
        let folded = fold_name(&service_value);
        let service = services
            .iter()
            .find(|service| service.active && (service.id == service_value || fold_name(&service.name) == folded))
            .ok_or_else(|| {
                AgentInputError::with_instruction(
                    "finance-service-not-found",
                    "serviceId",
                    "Usa el ID o el nombre exacto de un servicio activo (list_finance_services).",
                )
            })?;
        if transaction_type != "expense" || service.currency != currency_value {
            return Err(AgentInputError::with_instruction(
                "finance-service-expense-currency-mismatch",
                "serviceId",
                "Un servicio solo se paga con un gasto en su misma moneda.",
            ));
        }
        Some(service.clone())
    };
    Ok(TransactionDraft {
        transaction: FinanceTransaction {
            id: new_id(),
            transaction_type,
            amount: amount.unwrap_or_default(),
            currency: currency_value,
            effective_date: effective_date.unwrap_or_default(),
            account_id: account.id.clone(),
            destination_account_id: destination.map(|account| account.id.clone()),
            category_id: category.map(|category| category.id.clone()),
            description,
            source: String::new(),
            status: "confirmed".to_string(),
            actor_user_id: None,
            actor_library_user_id: None,
            source_artifact_id: None,
            service_id: service.as_ref().map(|service| service.id.clone()),
            merchant_id: None,
            operation_fingerprint: None,
            installment_id: None,
            source_reference: optional_text(args, "sourceReference"),
            raw_source: raw_text(args, "rawSource"),
            created_at: None,
            updated_at: None,
            purchase_date: None,
        },
        service,
    })
}

/// Movimiento vigente ya registrado con la misma evidencia de origen.
pub(crate) fn existing_transaction<'a>(
    transactions: &'a [FinanceTransaction],
    source_reference: &str,
) -> Option<&'a FinanceTransaction> {
    transactions.iter().find(|transaction| {
        transaction.status != "discarded" && transaction.source_reference.as_deref() == Some(source_reference)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, name: &str, currency: &str, account_type: &str) -> FinanceAccount {
        FinanceAccount {
            id: id.into(),
            name: name.into(),
            account_type: account_type.into(),
            currency: currency.into(),
            active: true,
        }
    }

    fn category(id: &str, name: &str, kind: &str) -> FinanceCategory {
        FinanceCategory {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            active: true,
            parent_id: None,
            description: None,
        }
    }

    #[test]
    fn normalizes_local_and_numeric_amounts() {
        assert_eq!(normalize_decimal(&json!("1.234,56"), 2).as_deref(), Some("1234.56"));
        assert_eq!(normalize_decimal(&json!("$ 1,234.50"), 2).as_deref(), Some("1234.5"));
        assert_eq!(normalize_decimal(&json!("1.234"), 2).as_deref(), Some("1234"));
        assert_eq!(normalize_decimal(&json!(12.5), 2).as_deref(), Some("12.5"));
        assert_eq!(normalize_decimal(&json!("0,5"), 6).as_deref(), Some("0.5"));
        assert_eq!(normalize_decimal(&json!("-5"), 2), None);
        assert_eq!(normalize_decimal(&json!(1.2345), 2), None);
        assert_eq!(normalize_decimal(&json!("abc"), 2), None);
    }

    #[test]
    fn sums_amounts_without_float_errors() {
        assert_eq!(sum_amounts(["0.1", "0.2", "10"]).as_deref(), Some("10.3"));
        assert_eq!(sum_amounts(["1.005"]), None);
    }

    #[test]
    fn category_reuses_an_existing_match_by_folded_name() {
        let categories = vec![category("cat-1", "Almacén", "expense")];
        let result = build_category(&json!({"name": "almacen", "kind": "expense"}), &categories)
            .expect("category");
        assert!(matches!(result, CreateOrExisting::Existing(existing) if existing.id == "cat-1"));
        assert!(build_category(&json!({"name": "x", "kind": "other"}), &categories).is_err());
    }

    #[test]
    fn purchase_resolves_account_by_name_and_fills_domain_fields() {
        let accounts = vec![account("acc-1", "Débito Galicia", "ARS", "bank")];
        let categories = vec![category("cat-1", "Supermercado", "expense")];
        let purchase = build_purchase(
            &json!({
                "accountId": "debito galicia",
                "categoryId": "Supermercado",
                "merchantName": "  Coto  ",
                "currency": "ars",
                "totalAmount": "1.500,00",
                "items": [{"originalDescription": "Pan", "quantity": 2, "unitPrice": "750", "lineTotal": "1500"}],
            }),
            &accounts,
            &categories,
            &[],
            "agent:request-1",
            "2026-09-22",
        )
        .expect("purchase");
        assert_eq!(purchase.account_id, "acc-1");
        assert_eq!(purchase.category_id.as_deref(), Some("cat-1"));
        assert_eq!(purchase.merchant_name, "Coto");
        assert_eq!(purchase.total_amount, "1500");
        assert_eq!(purchase.subtotal_amount, "1500");
        assert_eq!(purchase.status, "confirmed");
        assert_eq!(purchase.observed_at, "2026-09-22T12:00:00Z");
        assert_eq!(purchase.items[0].category_id.as_deref(), Some("cat-1"));
    }

    #[test]
    fn purchase_reports_every_invalid_field() {
        let error = build_purchase(&json!({"items": [{}]}), &[], &[], &[], "ref", "2026-09-22")
            .expect_err("invalid purchase");
        assert_eq!(error.error, "invalid-finance-purchase");
        for field in ["accountId", "merchantName", "currency", "totalAmount", "itemAmounts"] {
            assert!(error.invalid_fields.iter().any(|value| value == field), "{field}");
        }
    }

    #[test]
    fn purchase_names_the_line_to_fix_and_skips_voided_products() {
        let accounts = vec![account("acc-1", "Caja", "ARS", "cash")];
        let line = |description: &str, total: &str| json!({"originalDescription": description, "quantity": "1", "unitPrice": total, "lineTotal": total});
        let error = build_purchase(
            &json!({"accountId": "acc-1", "merchantName": "Coto", "currency": "ARS", "totalAmount": "1000",
                    "items": [line("Arroz", "1000"), line("ANULACION LECHE", "-1200"), {"originalDescription": "Pan", "quantity": "1"}]}),
            &accounts,
            &[],
            &[],
            "ref",
            "2026-09-22",
        )
        .expect_err("invalid lines");
        assert!(error.instruction.contains("items[1] («ANULACION LECHE»): unitPrice es negativo"), "{}", error.instruction);
        assert!(error.instruction.contains("voided: true"), "{}", error.instruction);
        assert!(error.instruction.contains("items[2] («Pan»): falta unitPrice, falta lineTotal"), "{}", error.instruction);
        let mut voided = line("Leche", "1200");
        voided["voided"] = json!(true);
        let purchase = build_purchase(
            &json!({"accountId": "acc-1", "merchantName": "Coto", "currency": "ARS", "totalAmount": "1000",
                    "items": [line("Arroz", "1000"), voided]}),
            &accounts,
            &[],
            &[],
            "ref",
            "2026-09-22",
        )
        .expect("voided product left out");
        assert_eq!((purchase.items.len(), purchase.subtotal_amount.as_str()), (1, "1000"));
    }

    #[test]
    fn purchase_rejects_an_account_in_another_currency() {
        let accounts = vec![account("acc-1", "Caja USD", "USD", "cash")];
        let error = build_purchase(
            &json!({"accountId": "acc-1", "merchantName": "M", "currency": "ARS", "totalAmount": "1",
                    "items": [{"originalDescription": "x", "quantity": "1", "unitPrice": "1", "lineTotal": "1"}]}),
            &accounts,
            &[],
            &[],
            "ref",
            "2026-09-22",
        )
        .expect_err("currency mismatch");
        assert_eq!(error.error, "finance-purchase-account-invalid");
    }

    #[test]
    fn salary_requires_a_pdf_for_signed_documents() {
        let accounts = vec![account("acc-1", "Sueldo", "ARS", "bank")];
        let args = json!({
            "accountId": "acc-1", "period": "2026-08", "paymentDate": "2026-09-01",
            "employer": "ACME", "grossAmount": "100", "deductionsTotal": "20", "netAmount": "80",
            "currency": "ARS", "signedDocument": true,
            "concepts": [{"name": "Básico", "conceptType": "earning", "amount": "100"}],
        });
        assert_eq!(
            build_salary(&args, &accounts, "chat:ref").expect_err("unsigned").error,
            "finance-salary-signed-pdf-required"
        );
        let salary = build_salary(&args, &accounts, "recibo.pdf").expect("salary");
        assert_eq!(salary.concepts.len(), 1);
        assert!(salary.signed_document);
    }

    #[test]
    fn statement_adds_synthetic_lines_for_aggregates_without_detail() {
        let accounts = vec![account("card-1", "Visa", "ARS", "credit_card")];
        let statement = build_credit_card_statement(
            &json!({
                "accountId": "Visa", "issuer": "Banco", "period": "2026-08",
                "closingDate": "2026-08-28", "dueDate": "2026-09-10", "currency": "ARS",
                "previousBalance": "0", "paymentsAmount": "0", "creditsAmount": "0",
                "purchasesAmount": "1000", "feesAmount": "50", "interestAmount": "0",
                "taxesAmount": "0", "totalDue": "1050",
                "items": [{"purchaseDate": "2026-08-10", "description": "Compra", "amount": "1000", "itemType": "purchase"}],
            }),
            &accounts,
            "ref",
        )
        .expect("statement");
        assert_eq!(statement.items.len(), 2);
        assert!(statement
            .items
            .iter()
            .any(|item| item.item_type == "fee" && item.amount == "50"));
    }

    fn transaction_fixtures() -> (Vec<FinanceAccount>, Vec<FinanceCategory>, Vec<FinanceService>) {
        let account = |id: &str, name: &str, currency: &str| FinanceAccount {
            id: id.into(),
            name: name.into(),
            account_type: "bank".into(),
            currency: currency.into(),
            active: true,
        };
        let accounts = vec![account("a1", "Banco Nación", "ARS"), account("a2", "Efectivo", "ARS"), account("a3", "Dólares", "USD")];
        let categories = vec![FinanceCategory {
            id: "c1".into(),
            name: "Supermercado".into(),
            kind: "expense".into(),
            active: true,
            parent_id: None,
            description: None,
        }];
        let services = vec![FinanceService {
            id: "s1".into(),
            name: "Luz".into(),
            category_id: "c1".into(),
            currency: "ARS".into(),
            expected_amount: "20000".into(),
            due_day: None,
            default_account_id: None,
            provider: None,
            modality: "fixed".into(),
            active: true,
            created_at: None,
            updated_at: None,
        }];
        (accounts, categories, services)
    }

    #[test]
    fn transactions_resolve_names_and_default_the_date_and_currency() {
        let (accounts, categories, services) = transaction_fixtures();
        let args = json!({
            "transactionType": "expense", "amount": "$ 12.345,50", "accountId": "banco nacion",
            "categoryId": "supermercado", "serviceId": "luz", "description": "Pago luz"
        });
        let draft = build_transaction(&args, &accounts, &categories, &services, "2026-09-22", true).expect("draft");
        assert_eq!(draft.transaction.amount, "12345.5");
        assert_eq!(draft.transaction.currency, "ARS");
        assert_eq!(draft.transaction.effective_date, "2026-09-22");
        assert_eq!(draft.transaction.account_id, "a1");
        assert_eq!(draft.transaction.category_id.as_deref(), Some("c1"));
        assert_eq!(draft.service.map(|service| service.id).as_deref(), Some("s1"));
    }

    #[test]
    fn transactions_reject_currency_transfer_and_category_mismatches() {
        let (accounts, categories, services) = transaction_fixtures();
        let build = |args: Value, require: bool| {
            build_transaction(&args, &accounts, &categories, &services, "2026-09-22", require).map(|_| ()).map_err(|error| error.error)
        };
        let base = json!({ "transactionType": "expense", "amount": "10", "accountId": "a1", "description": "x" });
        assert_eq!(build(base.clone(), true), Err("finance-expense-category-required"));
        assert_eq!(build(base.clone(), false), Ok(()));
        let mut usd = base.clone();
        usd["currency"] = json!("USD");
        assert_eq!(build(usd, false), Err("finance-account-currency-mismatch"));
        let transfer = json!({ "transactionType": "transfer", "amount": "10", "accountId": "a1", "destinationAccountId": "a3", "description": "x" });
        assert_eq!(build(transfer, false), Err("invalid-finance-transfer-account"));
        let negative = json!({ "transactionType": "expense", "amount": "-10", "accountId": "a1", "description": "x" });
        assert_eq!(build(negative, false), Err("invalid-finance-transaction"));
        let adjustment = json!({ "transactionType": "adjustment", "amount": "-10", "accountId": "a1", "description": "x" });
        assert_eq!(build(adjustment, false), Ok(()));
    }

    #[test]
    fn a_movement_splits_into_categories_that_add_up_to_its_total() {
        let order = json!({
            "transactionType": "expense", "amount": "12.500,50", "accountId": "a1", "description": "Pedido Rappi",
            "splits": [{ "amount": "10000", "categoryId": "Supermercado" }, { "amount": "2500.50", "categoryId": "Tabaco", "description": "Cigarrillos" }],
        });
        let parts = split_parts(&order).expect("valid").expect("split");
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0]["amount"].as_str(), parts[0]["categoryId"].as_str(), parts[0]["description"].as_str()), (Some("10000"), Some("Supermercado"), Some("Pedido Rappi")));
        assert_eq!((parts[1]["amount"].as_str(), parts[1]["description"].as_str()), (Some("2500.5"), Some("Cigarrillos")));
        assert!(parts.iter().all(|part| part.get("splits").is_none() && part["accountId"] == "a1"));
        assert_eq!(split_parts(&json!({ "amount": "10" })), Ok(None));
        let off = json!({ "amount": "100", "splits": [{ "amount": "60", "categoryId": "A" }, { "amount": "30", "categoryId": "B" }] });
        let error = split_parts(&off).expect_err("does not add up");
        assert!(error.instruction.contains("suman 90") && error.instruction.contains("100"), "{}", error.instruction);
        assert!(split_parts(&json!({ "amount": "10", "splits": [{ "amount": "10", "categoryId": "A" }] })).is_err(), "one part is no split");
        assert!(split_parts(&json!({ "amount": "10", "splits": [{ "amount": "5" }, { "amount": "5", "categoryId": "B" }] })).is_err());
    }
}
