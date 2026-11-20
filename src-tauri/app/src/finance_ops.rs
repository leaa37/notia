//! Escrituras de Finanzas: lo único que cambia los registros.
//!
//! Los formularios de la web y de la app, y las herramientas del asistente,
//! llaman a [`apply`] con un tipo (`expense`, `card-payment`, `loan`…), una
//! acción (`save` o `delete`) y valores con las claves de los formularios.
//! Lo que la persona puede corregir vuelve como [`OpError::Rejected`], con
//! el mensaje de cada campo; [`apply`] corre dentro de la transacción de
//! quien lo llama, así la vista previa del asistente lo prueba y lo deshace.
//!
//! Las referencias (`categoryId`, `cardId`, `serviceId`…) aceptan el id o
//! el nombre, para que el asistente no tenga que conocer ids.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::finance::{
    add_months, current_month, day_label, ensure_category, format_cents, money_text, month_label, new_id, now, today, typed_cents,
    valid_currency, valid_iso_date, valid_month, CATEGORY_LOANS, CATEGORY_OTHER, CATEGORY_OTHER_INCOME, CATEGORY_SALARY, CATEGORY_SERVICES,
};
use crate::finance_model::{self as model, cents};

pub(crate) type Values = Map<String, Value>;

const MAX_NAME_CHARS: usize = 120;
const MAX_NOTE_CHARS: usize = 500;
const MAX_CARD_INSTALLMENTS: u32 = 72;
const MAX_LOAN_INSTALLMENTS: u32 = 600;

#[derive(Debug)]
pub(crate) enum OpError {
    /// What the person can fix, by field key.
    Rejected { message: String, errors: BTreeMap<String, String> },
    Storage(String),
}

impl From<rusqlite::Error> for OpError {
    fn from(error: rusqlite::Error) -> Self {
        OpError::Storage(crate::finance::storage_message(&error))
    }
}

pub(crate) type OpResult<T> = Result<T, OpError>;

fn reject(key: &str, message: impl Into<String>) -> OpError {
    let message = message.into();
    OpError::Rejected { message: message.clone(), errors: BTreeMap::from([(key.to_string(), message)]) }
}

fn refuse(message: impl Into<String>) -> OpError {
    OpError::Rejected { message: message.into(), errors: BTreeMap::new() }
}

/// What applying something to Finanzas did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    /// For the person: «Gasto guardado: Nafta · $ 45.000».
    pub message: String,
    pub saved_id: Option<String>,
    /// How to take it back, when that is one call.
    pub undo: Option<Undo>,
}

/// A call to `finance_apply` that takes back what was just done.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Undo {
    pub kind: String,
    pub id: Option<String>,
    pub action: String,
    pub values: Values,
}

fn applied(message: impl Into<String>, saved_id: Option<String>) -> Applied {
    Applied { message: message.into(), saved_id, undo: None }
}

fn undo_delete(kind: &str, id: &str) -> Option<Undo> {
    Some(Undo { kind: kind.into(), id: Some(id.into()), action: "delete".into(), values: Map::new() })
}

// ---------------------------------------------------------------------------
// Reading the values
// ---------------------------------------------------------------------------

pub(crate) fn text(values: &Values, key: &str) -> Option<String> {
    match values.get(key) {
        Some(Value::String(text)) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn name(values: &Values, key: &str, what: &str) -> OpResult<String> {
    let value = text(values, key).ok_or_else(|| reject(key, format!("Escribí {what}.")))?;
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(reject(key, format!("El nombre puede tener hasta {MAX_NAME_CHARS} letras.")));
    }
    Ok(value)
}

fn amount(values: &Values, key: &str) -> OpResult<i128> {
    let value = values.get(key).filter(|value| !matches!(value, Value::Null) && value.as_str().is_none_or(|text| !text.trim().is_empty()));
    let Some(value) = value else {
        return Err(reject(key, "Escribí el importe."));
    };
    match typed_cents(value) {
        Some(cents) if cents > 0 => Ok(cents),
        _ => Err(reject(key, "El importe tiene que ser un número mayor que cero, como 1500 o 1.500,50.")),
    }
}

fn optional_amount(values: &Values, key: &str) -> OpResult<Option<i128>> {
    match values.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(_) => amount(values, key).map(Some),
    }
}

fn currency(values: &Values, key: &str, default: &str) -> OpResult<String> {
    let value = text(values, key).map(|value| value.to_uppercase()).unwrap_or_else(|| default.to_string());
    let value = match value.as_str() {
        "PESOS" | "$" => "ARS".to_string(),
        "DOLARES" | "DÓLARES" | "US$" | "U$S" => "USD".to_string(),
        _ => value,
    };
    if valid_currency(&value) {
        Ok(value)
    } else {
        Err(reject(key, "La moneda es pesos (ARS) o dólares (USD)."))
    }
}

fn date(values: &Values, key: &str) -> OpResult<String> {
    match text(values, key) {
        None => Ok(today()),
        Some(value) if valid_iso_date(&value) => Ok(value),
        Some(_) => Err(reject(key, "La fecha no es válida.")),
    }
}

fn month(values: &Values, key: &str, default: String) -> OpResult<String> {
    match text(values, key) {
        None => Ok(default),
        Some(value) if valid_month(&value) => Ok(value),
        Some(_) => Err(reject(key, "El mes no es válido.")),
    }
}

fn integer(values: &Values, key: &str) -> OpResult<Option<u32>> {
    match values.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number.as_u64().and_then(|value| u32::try_from(value).ok()).map(Some).ok_or_else(|| reject(key, "Escribí un número entero.")),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => text.trim().parse::<u32>().map(Some).map_err(|_| reject(key, "Escribí un número entero.")),
        Some(_) => Err(reject(key, "Escribí un número entero.")),
    }
}

/// Lowercase without accents, to match names.
pub(crate) fn fold(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// The id of the row of `table` named or identified by `value`.
fn find(connection: &Connection, table: &str, value: &str, extra: &str) -> OpResult<Option<String>> {
    let by_id: Option<String> = connection
        .query_row(&format!("SELECT id FROM {table} WHERE id=?1 {extra}"), [value], |row| row.get(0))
        .optional()?;
    if by_id.is_some() {
        return Ok(by_id);
    }
    let mut statement = connection.prepare(&format!("SELECT id,name FROM {table} WHERE 1=1 {extra}"))?;
    let wanted = fold(value);
    let found = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, name)| fold(name) == wanted)
        .map(|(id, _)| id);
    Ok(found)
}

fn names_of(connection: &Connection, table: &str, extra: &str) -> OpResult<String> {
    let mut statement = connection.prepare(&format!("SELECT name FROM {table} WHERE 1=1 {extra} ORDER BY name COLLATE NOCASE"))?;
    let names = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(names.join(", "))
}

/// A reference that must exist: by id or by name.
fn reference(connection: &Connection, values: &Values, key: &str, table: &str, extra: &str, what: &str) -> OpResult<String> {
    let Some(value) = text(values, key) else {
        return Err(reject(key, format!("Elegí {what}.")));
    };
    match find(connection, table, &value, extra)? {
        Some(id) => Ok(id),
        None => {
            let names = names_of(connection, table, extra)?;
            let known = if names.is_empty() { String::new() } else { format!(" Hay: {names}.") };
            Err(reject(key, format!("No encontré {what} «{value}».{known}")))
        }
    }
}

fn optional_reference(connection: &Connection, values: &Values, key: &str, table: &str, extra: &str, what: &str) -> OpResult<Option<String>> {
    if text(values, key).is_none() {
        return Ok(None);
    }
    reference(connection, values, key, table, extra, what).map(Some)
}

fn category(connection: &Connection, values: &Values, kind: &str, default: Option<(&str, &str)>) -> OpResult<String> {
    let extra = format!("AND kind='{kind}'");
    if text(values, "categoryId").is_none() {
        if let Some(default) = default {
            return Ok(ensure_category(connection, default, kind)?);
        }
    }
    reference(connection, values, "categoryId", "finance_categories", &extra, "la categoría")
}

fn account(connection: &Connection, values: &Values) -> OpResult<Option<String>> {
    optional_reference(connection, values, "accountId", "finance_accounts", "", "la cuenta")
}

fn name_of(connection: &Connection, table: &str, id: &str) -> OpResult<String> {
    Ok(connection.query_row(&format!("SELECT name FROM {table} WHERE id=?1"), [id], |row| row.get(0)).optional()?.unwrap_or_default())
}

fn require_id<'a>(id: Option<&'a str>, what: &str) -> OpResult<&'a str> {
    id.ok_or_else(|| refuse(format!("Falta {what}.")))
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

/// Applies `action` (`save` or `delete`) of `kind` to Finanzas, inside the
/// caller's transaction. `source` is who loads it (`manual`, `android`,
/// `telegram`…).
pub(crate) fn apply(connection: &Connection, source: &str, kind: &str, id: Option<&str>, action: &str, values: &Values) -> OpResult<Applied> {
    match (kind, action) {
        ("expense", "save") => save_movement(connection, source, "expense", id, values),
        ("income", "save") => save_movement(connection, source, "income", id, values),
        ("salary", "save") => save_salary(connection, source, id, values),
        ("movement", "save") => save_stored_movement(connection, source, require_id(id, "el movimiento")?, values),
        ("expense" | "income" | "salary" | "movement", "delete") => delete_movements(connection, &[require_id(id, "el movimiento")?.to_string()]),
        ("movements", "delete") => delete_movements(connection, &ids(values)),
        ("card-purchase", "save") => save_purchase(connection, id, values),
        ("card-purchase", "delete") => delete_row(connection, "finance_card_purchases", require_id(id, "la compra")?, "Compra borrada. Las cuotas ya pagadas quedan en Movimientos."),
        ("card-payment", "save") => pay_card(connection, source, values),
        ("loan-payment", "save") => pay_loan(connection, source, values),
        ("service-payment", "save") => pay_service(connection, source, values),
        ("savings" | "exchange", "save") => save_savings(connection, source, kind, id, values),
        ("savings" | "exchange", "delete") => delete_row(connection, "finance_savings_movements", require_id(id, "el movimiento de ahorro")?, "Movimiento de ahorro borrado."),
        ("category", "save") => save_category(connection, id, values),
        ("category", "delete") => delete_category(connection, require_id(id, "la categoría")?),
        ("account", "save") => save_named(connection, "finance_accounts", id, values, "el nombre de la cuenta", "Cuenta"),
        ("account", "delete") => delete_row(connection, "finance_accounts", require_id(id, "la cuenta")?, "Cuenta borrada. Sus movimientos quedan, sin cuenta."),
        ("card", "save") => save_named(connection, "finance_cards", id, values, "el nombre de la tarjeta", "Tarjeta"),
        ("card", "delete") => delete_row(connection, "finance_cards", require_id(id, "la tarjeta")?, "Tarjeta borrada, con sus compras. Las cuotas ya pagadas quedan en Movimientos."),
        ("service", "save") => save_service(connection, id, values),
        ("service", "delete") => delete_row(connection, "finance_services", require_id(id, "el servicio")?, "Servicio borrado. Sus pagos quedan en Movimientos."),
        ("loan", "save") => save_loan(connection, id, values),
        ("loan", "delete") => delete_row(connection, "finance_loans", require_id(id, "el préstamo")?, "Préstamo borrado. Las cuotas ya pagadas quedan en Movimientos."),
        ("budget", "save") => save_budget(connection, id, values),
        ("budget", "delete") => delete_row(connection, "finance_budgets", require_id(id, "el presupuesto")?, "Presupuesto borrado."),
        ("reserve", "save") => save_reserve(connection, id, values),
        ("reserve", "delete") => delete_row(connection, "finance_reserves", require_id(id, "la reserva")?, "Reserva borrada, con sus movimientos."),
        _ => Err(refuse("Esa operación de Finanzas no existe.")),
    }
}

fn ids(values: &Values) -> Vec<String> {
    values.get("ids").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn delete_row(connection: &Connection, table: &str, id: &str, message: &str) -> OpResult<Applied> {
    if connection.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])? == 0 {
        return Err(refuse("Ya no existe: tal vez se borró antes."));
    }
    Ok(applied(message, Some(id.to_string())))
}

// ---------------------------------------------------------------------------
// Movements
// ---------------------------------------------------------------------------

struct MovementInput {
    kind: &'static str,
    name: String,
    category_id: Option<String>,
    account_id: Option<String>,
    amount: i128,
    currency: String,
    date: String,
}

/// What a movement pays, if anything: a month of a service, or an
/// installment of a card purchase or of a loan.
#[derive(Default)]
struct PaysFor<'a> {
    service: Option<(&'a str, &'a str)>,
    purchase: Option<(&'a str, u32)>,
    loan: Option<(&'a str, u32)>,
}

fn insert_movement(connection: &Connection, source: &str, input: &MovementInput, pays: PaysFor<'_>) -> OpResult<String> {
    let id = new_id();
    let stamp = now();
    let (service_id, service_month) = pays.service.unzip();
    let purchase_id = pays.purchase.map(|(id, _)| id);
    let loan_id = pays.loan.map(|(id, _)| id);
    let number = pays.purchase.or(pays.loan).map(|(_, number)| number);
    connection.execute(
        "INSERT INTO finance_movements (id,kind,name,category_id,account_id,amount,currency,date,service_id,service_month,card_purchase_id,loan_id,installment_number,source,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)",
        params![
            id,
            input.kind,
            input.name,
            input.category_id,
            input.account_id,
            format_cents(input.amount),
            input.currency,
            input.date,
            service_id,
            service_month,
            purchase_id,
            loan_id,
            number,
            source,
            stamp
        ],
    )?;
    Ok(id)
}

fn kind_label(kind: &str) -> &'static str {
    if kind == "income" { "Ingreso" } else { "Gasto" }
}

fn save_movement(connection: &Connection, source: &str, kind: &'static str, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let default = if kind == "income" { Some(CATEGORY_OTHER_INCOME) } else { None };
    let input = MovementInput {
        kind,
        name: name(values, "name", if kind == "income" { "de qué es el ingreso" } else { "en qué gastaste" })?,
        amount: amount(values, "amount")?,
        currency: currency(values, "currency", "ARS")?,
        category_id: Some(category(connection, values, kind, default)?),
        account_id: account(connection, values)?,
        date: date(values, "date")?,
    };
    let category = name_of(connection, "finance_categories", input.category_id.as_deref().unwrap_or_default())?;
    let message = format!("{} guardado: {} · {} · {category}", kind_label(kind), input.name, money_text(input.amount, &input.currency));
    match id {
        Some(id) => {
            update_movement(connection, id, &input)?;
            Ok(applied(message, Some(id.to_string())))
        }
        None => {
            let id = insert_movement(connection, source, &input, PaysFor::default())?;
            Ok(Applied { undo: undo_delete("movement", &id), ..applied(message, Some(id)) })
        }
    }
}

fn update_movement(connection: &Connection, id: &str, input: &MovementInput) -> OpResult<()> {
    let changed = connection.execute(
        "UPDATE finance_movements SET name=?2,category_id=?3,account_id=?4,amount=?5,currency=?6,date=?7,updated_at=?8 WHERE id=?1",
        params![id, input.name, input.category_id, input.account_id, format_cents(input.amount), input.currency, input.date, now()],
    )?;
    if changed == 0 {
        return Err(refuse("El movimiento ya no existe."));
    }
    Ok(())
}

fn save_salary(connection: &Connection, source: &str, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let input = MovementInput {
        kind: "income",
        name: text(values, "name").unwrap_or_else(|| CATEGORY_SALARY.1.to_string()),
        amount: amount(values, "amount")?,
        currency: currency(values, "currency", "ARS")?,
        category_id: Some(ensure_category(connection, CATEGORY_SALARY, "income")?),
        account_id: account(connection, values)?,
        date: date(values, "date")?,
    };
    let message = format!("Sueldo guardado: {} del {}", money_text(input.amount, &input.currency), day_label(&input.date));
    match id {
        Some(id) => {
            update_movement(connection, id, &input)?;
            Ok(applied(message, Some(id.to_string())))
        }
        None => {
            let id = insert_movement(connection, source, &input, PaysFor::default())?;
            Ok(Applied { undo: undo_delete("movement", &id), ..applied(message, Some(id)) })
        }
    }
}

/// Edits a stored movement. A payment keeps what it pays (and its currency);
/// the rest changes as in any movement.
fn save_stored_movement(connection: &Connection, source: &str, id: &str, values: &Values) -> OpResult<Applied> {
    let stored = model::load_movement(connection, id)?.ok_or_else(|| refuse("El movimiento ya no existe."))?;
    let is_payment = stored.card_purchase_id.is_some() || stored.loan_id.is_some() || stored.service_id.is_some();
    if !is_payment {
        if stored.kind == "income" && stored.category_id.as_deref() == Some(CATEGORY_SALARY.0) && text(values, "categoryId").is_none() {
            return save_salary(connection, source, Some(id), values);
        }
        let kind = if stored.kind == "income" { "income" } else { "expense" };
        return save_movement(connection, source, kind, Some(id), values);
    }
    let input = MovementInput {
        kind: "expense",
        name: text(values, "name").unwrap_or(stored.name.clone()),
        amount: amount(values, "amount")?,
        currency: stored.currency.clone(),
        category_id: match text(values, "categoryId") {
            Some(_) => Some(category(connection, values, "expense", None)?),
            None => stored.category_id.clone(),
        },
        account_id: account(connection, values)?,
        date: date(values, "date")?,
    };
    update_movement(connection, id, &input)?;
    Ok(applied(format!("Pago guardado: {} · {}", input.name, money_text(input.amount, &input.currency)), Some(id.to_string())))
}

fn delete_movements(connection: &Connection, ids: &[String]) -> OpResult<Applied> {
    if ids.is_empty() {
        return Err(refuse("No hay nada para borrar."));
    }
    let mut deleted = 0;
    for id in ids {
        deleted += connection.execute("DELETE FROM finance_movements WHERE id=?1", [id])?;
    }
    if deleted == 0 {
        return Err(refuse("Ya no existe: tal vez se borró antes."));
    }
    let message = if deleted == 1 { "Movimiento borrado.".to_string() } else { format!("{deleted} movimientos borrados.") };
    Ok(applied(message, ids.first().cloned()))
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

/// The card of a purchase: the one chosen, or a new one by its name when
/// there were none (`newCard`).
fn purchase_card(connection: &Connection, values: &Values) -> OpResult<String> {
    if text(values, "cardId").is_none() {
        if let Some(name) = text(values, "newCard") {
            if let Some(id) = find(connection, "finance_cards", &name, "")? {
                return Ok(id);
            }
            if name.chars().count() > MAX_NAME_CHARS {
                return Err(reject("newCard", format!("El nombre puede tener hasta {MAX_NAME_CHARS} letras.")));
            }
            return upsert_named(connection, "finance_cards", None, &name);
        }
    }
    reference(connection, values, "cardId", "finance_cards", "", "la tarjeta")
}

fn save_purchase(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let card_id = purchase_card(connection, values)?;
    let name = name(values, "name", "qué compraste")?;
    let total = amount(values, "total")?;
    let currency = currency(values, "currency", "ARS")?;
    let count = integer(values, "installmentCount")?.unwrap_or(1);
    if !(1..=MAX_CARD_INSTALLMENTS).contains(&count) {
        return Err(reject("installmentCount", format!("Las cuotas van de 1 a {MAX_CARD_INSTALLMENTS}.")));
    }
    let category_id = category(connection, values, "expense", Some(CATEGORY_OTHER))?;
    let first_month = month(values, "firstMonth", add_months(&current_month(), 1))?;
    if let Some(id) = id {
        let state = model::purchase_state_of(connection, id)?.ok_or_else(|| refuse("La compra ya no existe."))?;
        let last_paid = state.installments.iter().filter(|installment| installment.paid.is_some()).map(|installment| installment.number).max().unwrap_or(0);
        if count < last_paid {
            return Err(reject("installmentCount", format!("Ya pagaste la cuota {last_paid}: no puede tener menos cuotas.")));
        }
        if state.purchase.currency != currency && state.paid_count > 0 {
            return Err(reject("currency", "Ya tiene cuotas pagadas: la moneda no puede cambiar."));
        }
        connection.execute(
            "UPDATE finance_card_purchases SET card_id=?2,name=?3,category_id=?4,currency=?5,total=?6,installment_count=?7,first_month=?8,updated_at=?9 WHERE id=?1",
            params![id, card_id, name, category_id, currency, format_cents(total), count, first_month, now()],
        )?;
        return Ok(applied(format!("Compra guardada: {name}."), Some(id.to_string())));
    }
    let id = new_id();
    let stamp = now();
    connection.execute(
        "INSERT INTO finance_card_purchases (id,card_id,name,category_id,currency,total,installment_count,first_month,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
        params![id, card_id, name, category_id, currency, format_cents(total), count, first_month, stamp],
    )?;
    let card = name_of(connection, "finance_cards", &card_id)?;
    let each = crate::finance::installment_amounts(total, count);
    let plan = if count == 1 {
        format!("en 1 pago, en el resumen de {}", month_label(&first_month))
    } else {
        format!("en {count} cuotas de {} desde {}", money_text(*each.last().unwrap_or(&total), &currency), month_label(&first_month))
    };
    Ok(Applied {
        undo: undo_delete("card-purchase", &id),
        ..applied(format!("Compra con {card} guardada: {name} · {} {plan}.", money_text(total, &currency)), Some(id))
    })
}

/// `purchaseId:number` of each installment to pay.
fn installment_refs(values: &Values) -> Vec<(String, u32)> {
    let list = match values.get("installments") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>(),
        Some(Value::String(text)) => text.split(',').map(|item| item.trim().to_string()).collect(),
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|item| {
            let (purchase, number) = item.rsplit_once(':')?;
            Some((purchase.to_string(), number.parse().ok()?))
        })
        .collect()
}

/// Pays installments of a card: the ones chosen, or every unpaid one due up
/// to the month (`month`, this one by default). Each installment becomes an
/// expense of its purchase's category, dated the day of the payment.
fn pay_card(connection: &Connection, source: &str, values: &Values) -> OpResult<Applied> {
    let card_id = reference(connection, values, "cardId", "finance_cards", "", "la tarjeta")?;
    let card = name_of(connection, "finance_cards", &card_id)?;
    let states = model::purchase_states(connection)?;
    let until = month(values, "month", current_month())?;
    let chosen = installment_refs(values);
    let wanted: Vec<(String, u32)> = if values.contains_key("installments") {
        if chosen.is_empty() {
            return Err(reject("installments", "Elegí al menos una cuota."));
        }
        chosen
    } else {
        model::card_installments(&states, &card_id, &until)
            .into_iter()
            .filter(|installment| installment.paid.is_none())
            .map(|installment| (installment.purchase_id, installment.number))
            .collect()
    };
    if wanted.is_empty() {
        return Err(refuse(format!("{card} no tiene cuotas para pagar hasta {}.", month_label(&until))));
    }
    let date = date(values, "date")?;
    let account_id = account(connection, values)?;
    let mut created: Vec<(String, u32)> = Vec::new();
    let mut ids = Vec::new();
    let mut totals: BTreeMap<String, i128> = BTreeMap::new();
    for (purchase_id, number) in &wanted {
        let state = states
            .iter()
            .find(|state| &state.purchase.id == purchase_id && state.purchase.card_id == card_id)
            .ok_or_else(|| reject("installments", "Esa cuota no es de esta tarjeta."))?;
        let installment = state.installment(*number).ok_or_else(|| reject("installments", "Esa cuota no existe."))?;
        let label = format!("{} · cuota {}/{}", state.purchase.name, number, state.purchase.installment_count);
        if installment.paid.is_some() || created.iter().any(|(id, n)| id == purchase_id && n == number) {
            return Err(reject("installments", format!("La {label} ya está pagada.")));
        }
        let name = if state.purchase.installment_count == 1 { state.purchase.name.clone() } else { label };
        let input = MovementInput {
            kind: "expense",
            name,
            category_id: state.purchase.category_id.clone(),
            account_id: account_id.clone(),
            amount: cents(&installment.amount),
            currency: state.purchase.currency.clone(),
            date: date.clone(),
        };
        let id = insert_movement(connection, source, &input, PaysFor { purchase: Some((purchase_id, *number)), ..PaysFor::default() })?;
        *totals.entry(input.currency.clone()).or_default() += input.amount;
        created.push((purchase_id.clone(), *number));
        ids.push(id);
    }
    let total = totals.iter().map(|(currency, cents)| money_text(*cents, currency)).collect::<Vec<_>>().join(" + ");
    let message = if ids.len() == 1 {
        format!("Cuota de {card} pagada: {total}.")
    } else {
        format!("{} cuotas de {card} pagadas: {total}.", ids.len())
    };
    let undo = Undo { kind: "movements".into(), id: None, action: "delete".into(), values: Map::from_iter([("ids".to_string(), json!(ids))]) };
    Ok(Applied { message, saved_id: ids.first().cloned(), undo: Some(undo) })
}

// ---------------------------------------------------------------------------
// Loans and services
// ---------------------------------------------------------------------------

fn pay_loan(connection: &Connection, source: &str, values: &Values) -> OpResult<Applied> {
    let loan_id = reference(connection, values, "loanId", "finance_loans", "", "el préstamo")?;
    let state = model::loan_state_of(connection, &loan_id)?.ok_or_else(|| refuse("El préstamo ya no existe."))?;
    let number = match integer(values, "number")? {
        Some(number) => number,
        None => state.next_number.ok_or_else(|| refuse(format!("{} ya está pagado entero.", state.loan.name)))?,
    };
    let count = state.loan.installment_count;
    let installment = state.installments.iter().find(|installment| installment.number == number).ok_or_else(|| reject("number", format!("Las cuotas van de 1 a {count}.")))?;
    if let Some(paid) = &installment.paid {
        return Err(reject("number", format!("La cuota {number} ya está pagada (el {}).", day_label(&paid.date))));
    }
    let input = MovementInput {
        kind: "expense",
        name: format!("{} · cuota {number}/{count}", state.loan.name),
        category_id: Some(ensure_category(connection, CATEGORY_LOANS, "expense")?),
        account_id: account(connection, values)?,
        amount: amount(values, "amount")?,
        currency: state.loan.currency.clone(),
        date: date(values, "date")?,
    };
    let id = insert_movement(connection, source, &input, PaysFor { loan: Some((&loan_id, number)), ..PaysFor::default() })?;
    let left = count - state.paid_count - 1;
    let rest = if left == 0 { "Con esta terminaste el préstamo.".to_string() } else { format!("Quedan {left}.") };
    Ok(Applied {
        undo: undo_delete("movement", &id),
        ..applied(format!("Cuota {number}/{count} de {} pagada: {}. {rest}", state.loan.name, money_text(input.amount, &input.currency)), Some(id))
    })
}

fn pay_service(connection: &Connection, source: &str, values: &Values) -> OpResult<Applied> {
    let service_id = reference(connection, values, "serviceId", "finance_services", "", "el servicio")?;
    let service = model::list_services(connection)?.into_iter().find(|service| service.id == service_id).ok_or_else(|| refuse("El servicio ya no existe."))?;
    let month = month(values, "month", current_month())?;
    let paid: Option<String> = connection
        .query_row("SELECT date FROM finance_movements WHERE service_id=?1 AND service_month=?2", params![service_id, month], |row| row.get(0))
        .optional()?;
    if let Some(paid) = paid {
        return Err(reject("month", format!("{} de {} ya está pagado (el {}).", service.name, month_label(&month), day_label(&paid))));
    }
    let amount = match optional_amount(values, "amount")? {
        Some(amount) => amount,
        None => cents(&model::service_estimate(connection, &service)?),
    };
    let input = MovementInput {
        kind: "expense",
        name: service.name.clone(),
        category_id: Some(ensure_category(connection, CATEGORY_SERVICES, "expense")?),
        account_id: account(connection, values)?,
        amount,
        currency: service.currency.clone(),
        date: date(values, "date")?,
    };
    let id = insert_movement(connection, source, &input, PaysFor { service: Some((&service_id, &month)), ..PaysFor::default() })?;
    Ok(Applied {
        undo: undo_delete("movement", &id),
        ..applied(format!("{} de {} pagado: {}.", service.name, month_label(&month), money_text(amount, &service.currency)), Some(id))
    })
}

// ---------------------------------------------------------------------------
// Savings
// ---------------------------------------------------------------------------

fn save_savings(connection: &Connection, source: &str, kind: &str, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let date = date(values, "date")?;
    let note = text(values, "note").unwrap_or_default();
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(reject("note", format!("La nota puede tener hasta {MAX_NOTE_CHARS} letras.")));
    }
    let (reserve_id, movement_kind, amount, pesos, message) = if kind == "exchange" {
        let selling = text(values, "type").as_deref() == Some("sell");
        let reserve_id = match text(values, "reserveId") {
            Some(_) => reference(connection, values, "reserveId", "finance_reserves", "AND currency='USD'", "la reserva en dólares")?,
            None => find_first_reserve(connection, "USD")?.ok_or_else(|| reject("reserveId", "Creá primero una reserva en dólares."))?,
        };
        let dollars = amount(values, "dollars")?;
        let pesos = amount(values, "pesos")?;
        let rate = money_text(pesos * 100 / dollars, "ARS");
        let verb = if selling { "Vendiste" } else { "Compraste" };
        let message = format!("{verb} {} por {} ({rate} cada dólar).", money_text(dollars, "USD"), money_text(pesos, "ARS"));
        (reserve_id, if selling { "withdrawal" } else { "deposit" }, dollars, Some(pesos), message)
    } else {
        let reserve_id = reference(connection, values, "reserveId", "finance_reserves", "", "la reserva")?;
        let withdrawal = text(values, "type").as_deref() == Some("withdrawal");
        let amount = amount(values, "amount")?;
        let reserve = name_of(connection, "finance_reserves", &reserve_id)?;
        let currency: String = connection.query_row("SELECT currency FROM finance_reserves WHERE id=?1", [&reserve_id], |row| row.get(0))?;
        let message = if withdrawal {
            format!("Sacaste {} de {reserve}.", money_text(amount, &currency))
        } else {
            format!("Guardaste {} en {reserve}.", money_text(amount, &currency))
        };
        (reserve_id, if withdrawal { "withdrawal" } else { "deposit" }, amount, None, message)
    };
    if movement_kind == "withdrawal" {
        let balance = model::reserve_balances(connection)?.get(&reserve_id).copied().unwrap_or(0);
        let previous = match id {
            Some(id) => model::load_savings_movement(connection, id)?
                .map(|movement| if movement.kind == "withdrawal" { cents(&movement.amount) } else { -cents(&movement.amount) })
                .unwrap_or(0),
            None => 0,
        };
        if amount > balance + previous {
            let field = if kind == "exchange" { "dollars" } else { "amount" };
            return Err(reject(field, "La reserva no tiene tanto."));
        }
    }
    let pesos = pesos.map(format_cents);
    match id {
        Some(id) => {
            let changed = connection.execute(
                "UPDATE finance_savings_movements SET reserve_id=?2,kind=?3,amount=?4,pesos=?5,date=?6,note=?7,updated_at=?8 WHERE id=?1",
                params![id, reserve_id, movement_kind, format_cents(amount), pesos, date, note, now()],
            )?;
            if changed == 0 {
                return Err(refuse("El movimiento de ahorro ya no existe."));
            }
            Ok(applied(message, Some(id.to_string())))
        }
        None => {
            let id = new_id();
            connection.execute(
                "INSERT INTO finance_savings_movements (id,reserve_id,kind,amount,pesos,date,note,source,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
                params![id, reserve_id, movement_kind, format_cents(amount), pesos, date, note, source, now()],
            )?;
            Ok(Applied { undo: undo_delete(kind, &id), ..applied(message, Some(id)) })
        }
    }
}

fn find_first_reserve(connection: &Connection, currency: &str) -> OpResult<Option<String>> {
    Ok(connection
        .query_row("SELECT id FROM finance_reserves WHERE currency=?1 ORDER BY created_at, name LIMIT 1", [currency], |row| row.get(0))
        .optional()?)
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

/// Rejects a name that another row of `table` already has.
fn unique_name(connection: &Connection, table: &str, id: Option<&str>, name: &str, extra: &str) -> OpResult<()> {
    if let Some(other) = find(connection, table, name, extra)? {
        if Some(other.as_str()) != id && fold(&name_of(connection, table, &other)?) == fold(name) {
            return Err(reject("name", format!("Ya hay una con el nombre «{name}».")));
        }
    }
    Ok(())
}

fn upsert_named(connection: &Connection, table: &str, id: Option<&str>, name: &str) -> OpResult<String> {
    let stamp = now();
    match id {
        Some(id) => {
            if connection.execute(&format!("UPDATE {table} SET name=?2,updated_at=?3 WHERE id=?1"), params![id, name, stamp])? == 0 {
                return Err(refuse("Ya no existe: tal vez se borró antes."));
            }
            Ok(id.to_string())
        }
        None => {
            let id = new_id();
            connection.execute(&format!("INSERT INTO {table} (id,name,created_at,updated_at) VALUES (?1,?2,?3,?3)"), params![id, name, stamp])?;
            Ok(id)
        }
    }
}

fn save_named(connection: &Connection, table: &str, id: Option<&str>, values: &Values, what: &str, label: &str) -> OpResult<Applied> {
    let name = name(values, "name", what)?;
    unique_name(connection, table, id, &name, "")?;
    let saved = upsert_named(connection, table, id, &name)?;
    Ok(applied(format!("{label} guardada: {name}."), Some(saved)))
}

fn save_category(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let name = name(values, "name", "el nombre de la categoría")?;
    let kind = match text(values, "kind").as_deref() {
        None | Some("expense") => "expense",
        Some("income") => "income",
        Some(_) => return Err(reject("kind", "Una categoría es de gastos o de ingresos.")),
    };
    if let Some(id) = id {
        let stored: Option<String> = connection.query_row("SELECT kind FROM finance_categories WHERE id=?1", [id], |row| row.get(0)).optional()?;
        let stored = stored.ok_or_else(|| refuse("La categoría ya no existe."))?;
        if stored != kind {
            let used: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM finance_movements WHERE category_id=?1)", [id], |row| row.get(0))?;
            if used {
                return Err(reject("kind", "Ya tiene movimientos: no puede pasar de gastos a ingresos."));
            }
        }
    }
    unique_name(connection, "finance_categories", id, &name, &format!("AND kind='{kind}'"))?;
    let stamp = now();
    let saved = match id {
        Some(id) => {
            connection.execute("UPDATE finance_categories SET name=?2,kind=?3,updated_at=?4 WHERE id=?1", params![id, name, kind, stamp])?;
            id.to_string()
        }
        None => {
            let id = new_id();
            connection.execute("INSERT INTO finance_categories (id,name,kind,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)", params![id, name, kind, stamp])?;
            id
        }
    };
    Ok(applied(format!("Categoría guardada: {name}."), Some(saved)))
}

fn delete_category(connection: &Connection, id: &str) -> OpResult<Applied> {
    let used: i64 = connection.query_row("SELECT COUNT(*) FROM finance_movements WHERE category_id=?1", [id], |row| row.get(0))?;
    connection.execute("UPDATE finance_card_purchases SET category_id=NULL WHERE category_id=?1", [id])?;
    let mut message = "Categoría borrada.".to_string();
    if used > 0 {
        message.push_str(&format!(" Sus {used} movimientos quedan sin categoría."));
    }
    delete_row(connection, "finance_categories", id, &message)
}

fn save_service(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let name = name(values, "name", "el nombre del servicio")?;
    let amount = amount(values, "amount")?;
    let currency = currency(values, "currency", "ARS")?;
    unique_name(connection, "finance_services", id, &name, "")?;
    let stamp = now();
    let saved = match id {
        Some(id) => {
            if connection.execute("UPDATE finance_services SET name=?2,currency=?3,amount=?4,updated_at=?5 WHERE id=?1", params![id, name, currency, format_cents(amount), stamp])? == 0 {
                return Err(refuse("El servicio ya no existe."));
            }
            id.to_string()
        }
        None => {
            let id = new_id();
            connection.execute(
                "INSERT INTO finance_services (id,name,currency,amount,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
                params![id, name, currency, format_cents(amount), stamp],
            )?;
            id
        }
    };
    Ok(applied(format!("Servicio guardado: {name} · {} por mes.", money_text(amount, &currency)), Some(saved)))
}

fn save_loan(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let name = name(values, "name", "el nombre del préstamo")?;
    let amount = amount(values, "amount")?;
    let currency = currency(values, "currency", "ARS")?;
    let count = integer(values, "installmentCount")?.ok_or_else(|| reject("installmentCount", "Escribí cuántas cuotas tiene."))?;
    if !(1..=MAX_LOAN_INSTALLMENTS).contains(&count) {
        return Err(reject("installmentCount", format!("Las cuotas van de 1 a {MAX_LOAN_INSTALLMENTS}.")));
    }
    let first_month = month(values, "firstMonth", add_months(&current_month(), 1))?;
    unique_name(connection, "finance_loans", id, &name, "")?;
    let stamp = now();
    let saved = match id {
        Some(id) => {
            let state = model::loan_state_of(connection, id)?.ok_or_else(|| refuse("El préstamo ya no existe."))?;
            let last_paid = state.installments.iter().filter(|installment| installment.paid.is_some()).map(|installment| installment.number).max().unwrap_or(0);
            if count < last_paid {
                return Err(reject("installmentCount", format!("Ya pagaste la cuota {last_paid}: no puede tener menos cuotas.")));
            }
            if state.loan.currency != currency && state.paid_count > 0 {
                return Err(reject("currency", "Ya tiene cuotas pagadas: la moneda no puede cambiar."));
            }
            connection.execute(
                "UPDATE finance_loans SET name=?2,currency=?3,amount=?4,installment_count=?5,first_month=?6,updated_at=?7 WHERE id=?1",
                params![id, name, currency, format_cents(amount), count, first_month, stamp],
            )?;
            id.to_string()
        }
        None => {
            let id = new_id();
            connection.execute(
                "INSERT INTO finance_loans (id,name,currency,amount,installment_count,first_month,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
                params![id, name, currency, format_cents(amount), count, first_month, stamp],
            )?;
            id
        }
    };
    Ok(applied(format!("Préstamo guardado: {name} · {} en {count} cuotas.", money_text(amount, &currency)), Some(saved)))
}

/// A budget per category and currency: saving one for a category that has
/// it replaces its amount.
fn save_budget(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let category_id = category(connection, values, "expense", None)?;
    let amount = amount(values, "amount")?;
    let currency = currency(values, "currency", "ARS")?;
    let existing: Option<String> = connection
        .query_row("SELECT id FROM finance_budgets WHERE category_id=?1 AND currency=?2", params![category_id, currency], |row| row.get(0))
        .optional()?;
    let stamp = now();
    let target = match (id, existing) {
        (Some(id), Some(other)) if other != id => {
            return Err(reject("categoryId", "Esa categoría ya tiene presupuesto en esa moneda: cambialo ahí."));
        }
        (Some(id), _) => Some(id.to_string()),
        (None, existing) => existing,
    };
    let saved = match target {
        Some(id) => {
            if connection.execute(
                "UPDATE finance_budgets SET category_id=?2,currency=?3,amount=?4,updated_at=?5 WHERE id=?1",
                params![id, category_id, currency, format_cents(amount), stamp],
            )? == 0
            {
                return Err(refuse("El presupuesto ya no existe."));
            }
            id
        }
        None => {
            let id = new_id();
            connection.execute(
                "INSERT INTO finance_budgets (id,category_id,currency,amount,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
                params![id, category_id, currency, format_cents(amount), stamp],
            )?;
            id
        }
    };
    connection.execute("DELETE FROM finance_budget_alerts WHERE budget_id=?1", [&saved])?;
    let category = name_of(connection, "finance_categories", &category_id)?;
    Ok(applied(format!("Presupuesto de {category}: {} por mes.", money_text(amount, &currency)), Some(saved)))
}

fn save_reserve(connection: &Connection, id: Option<&str>, values: &Values) -> OpResult<Applied> {
    let name = name(values, "name", "el nombre de la reserva")?;
    let currency = currency(values, "currency", "ARS")?;
    let goal = optional_amount(values, "goal")?.map(format_cents);
    unique_name(connection, "finance_reserves", id, &name, "")?;
    let stamp = now();
    let saved = match id {
        Some(id) => {
            let stored: Option<String> = connection.query_row("SELECT currency FROM finance_reserves WHERE id=?1", [id], |row| row.get(0)).optional()?;
            let stored = stored.ok_or_else(|| refuse("La reserva ya no existe."))?;
            let used: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM finance_savings_movements WHERE reserve_id=?1)", [id], |row| row.get(0))?;
            if stored != currency && used {
                return Err(reject("currency", "Ya tiene movimientos: la moneda no puede cambiar."));
            }
            connection.execute("UPDATE finance_reserves SET name=?2,currency=?3,goal=?4,updated_at=?5 WHERE id=?1", params![id, name, currency, goal, stamp])?;
            id.to_string()
        }
        None => {
            let id = new_id();
            connection.execute(
                "INSERT INTO finance_reserves (id,name,currency,goal,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
                params![id, name, currency, goal, stamp],
            )?;
            id
        }
    };
    Ok(applied(format!("Reserva guardada: {name}."), Some(saved)))
}

