//! Herramientas del asistente sobre Finanzas (chat de Finanzas, chat de la
//! biblioteca y Telegram).
//!
//! Las escrituras pasan por [`finance_ops::apply`], igual que los
//! formularios: la vista previa la aplica en una transacción y la deshace,
//! así lo que se confirma es exactamente lo que se guarda. Las referencias
//! van por nombre (la categoría, la tarjeta, el servicio…); el asistente no
//! necesita ids salvo para editar o borrar algo que ya leyó.

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::finance::{current_month, valid_month, write, FinanceContext};
use crate::finance_model as model;
use crate::finance_ops::{self as ops, fold, OpError, Values};
use crate::finance_screen::{self as screen, MovementFilter};

pub const FINANCE_READ_TOOLS: [&str; 7] = [
    "get_finance_month",
    "get_finance_payments",
    "list_finance_movements",
    "get_finance_setup",
    "get_finance_dollar_quotes",
    "get_finance_inflation_indices",
    "get_finance_historical_dollar_quotes",
];

pub const FINANCE_WRITE_TOOLS: [&str; 11] = [
    "save_finance_expense",
    "save_finance_income",
    "save_finance_salary",
    "save_finance_card_purchase",
    "pay_finance_card",
    "pay_finance_loan_installment",
    "pay_finance_service",
    "save_finance_savings_movement",
    "save_finance_dollar_exchange",
    "save_finance_setup",
    "delete_finance_record",
];

pub fn is_finance_tool(name: &str) -> bool {
    FINANCE_READ_TOOLS.contains(&name) || is_finance_write_tool(name)
}

pub fn is_finance_write_tool(name: &str) -> bool {
    FINANCE_WRITE_TOOLS.contains(&name)
}

#[derive(Debug)]
pub(crate) enum ToolFailure {
    /// What the model can fix (or ask the person).
    Invalid(String),
    /// Storage, permissions or an external service.
    Unavailable(String),
}

impl From<rusqlite::Error> for ToolFailure {
    fn from(error: rusqlite::Error) -> Self {
        ToolFailure::Unavailable(crate::finance::storage_message(&error))
    }
}

impl From<crate::finance::FinanceCommandError> for ToolFailure {
    fn from(error: crate::finance::FinanceCommandError) -> Self {
        match error.code {
            "storage" => ToolFailure::Unavailable(error.message),
            _ => ToolFailure::Invalid(error.message),
        }
    }
}

type ToolResult<T> = Result<T, ToolFailure>;

fn rejection(message: String, errors: std::collections::BTreeMap<String, String>) -> ToolFailure {
    if errors.is_empty() {
        return ToolFailure::Invalid(message);
    }
    let fields = errors.iter().map(|(field, text)| format!("{field}: {text}")).collect::<Vec<_>>().join(" ");
    ToolFailure::Invalid(format!("{message} ({fields})"))
}

fn text(arguments: &Value, key: &str) -> Option<String> {
    match arguments.get(key) {
        Some(Value::String(text)) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

/// Copies `from` of the arguments to `to` of the values.
fn copy(values: &mut Values, arguments: &Value, pairs: &[(&str, &str)]) {
    for (from, to) in pairs {
        if let Some(value) = arguments.get(*from).filter(|value| !value.is_null()) {
            values.insert((*to).to_string(), value.clone());
        }
    }
}

/// What a write tool asks of Finanzas: the kind, the id, the action and the
/// values in the forms' keys.
fn request(connection: &Connection, name: &str, arguments: &Value) -> ToolResult<(String, Option<String>, String, Values)> {
    let mut values = Map::new();
    let id = text(arguments, "id");
    let common = [("amount", "amount"), ("currency", "currency"), ("date", "date"), ("account", "accountId")];
    let (kind, action) = match name {
        "save_finance_expense" => {
            copy(&mut values, arguments, &[("name", "name"), ("category", "categoryId")]);
            copy(&mut values, arguments, &common);
            ("expense".to_string(), "save")
        }
        "save_finance_income" => {
            copy(&mut values, arguments, &[("name", "name"), ("category", "categoryId")]);
            copy(&mut values, arguments, &common);
            ("income".to_string(), "save")
        }
        "save_finance_salary" => {
            copy(&mut values, arguments, &common);
            ("salary".to_string(), "save")
        }
        "save_finance_card_purchase" => {
            copy(
                &mut values,
                arguments,
                &[("card", "cardId"), ("name", "name"), ("total", "total"), ("currency", "currency"), ("installments", "installmentCount"), ("category", "categoryId"), ("firstMonth", "firstMonth")],
            );
            ("card-purchase".to_string(), "save")
        }
        "pay_finance_card" => {
            copy(&mut values, arguments, &[("card", "cardId"), ("month", "month"), ("date", "date"), ("account", "accountId")]);
            if let Some(chosen) = arguments.get("installments").and_then(Value::as_array) {
                values.insert("installments".into(), Value::Array(installment_refs(connection, chosen)?));
            }
            ("card-payment".to_string(), "save")
        }
        "pay_finance_loan_installment" => {
            copy(&mut values, arguments, &[("loan", "loanId"), ("number", "number"), ("amount", "amount"), ("date", "date"), ("account", "accountId")]);
            ("loan-payment".to_string(), "save")
        }
        "pay_finance_service" => {
            copy(&mut values, arguments, &[("service", "serviceId"), ("month", "month"), ("amount", "amount"), ("date", "date"), ("account", "accountId")]);
            ("service-payment".to_string(), "save")
        }
        "save_finance_savings_movement" => {
            copy(&mut values, arguments, &[("reserve", "reserveId"), ("type", "type"), ("amount", "amount"), ("date", "date"), ("note", "note")]);
            ("savings".to_string(), "save")
        }
        "save_finance_dollar_exchange" => {
            copy(&mut values, arguments, &[("type", "type"), ("dollars", "dollars"), ("pesos", "pesos"), ("reserve", "reserveId"), ("date", "date")]);
            ("exchange".to_string(), "save")
        }
        "save_finance_setup" => {
            let kind = text(arguments, "kind").ok_or_else(|| ToolFailure::Invalid("Falta kind: card, service, loan, budget, category, reserve o account.".into()))?;
            if !["card", "service", "loan", "budget", "category", "reserve", "account"].contains(&kind.as_str()) {
                return Err(ToolFailure::Invalid("kind tiene que ser card, service, loan, budget, category, reserve o account.".into()));
            }
            copy(
                &mut values,
                arguments,
                &[
                    ("name", "name"),
                    ("amount", "amount"),
                    ("currency", "currency"),
                    ("installments", "installmentCount"),
                    ("firstMonth", "firstMonth"),
                    ("category", "categoryId"),
                    ("categoryKind", "kind"),
                    ("goal", "goal"),
                ],
            );
            (kind, "save")
        }
        "delete_finance_record" => {
            let kind = text(arguments, "kind").ok_or_else(|| ToolFailure::Invalid("Falta kind.".into()))?;
            let kind = match kind.as_str() {
                "movement" | "card-purchase" | "savings" | "card" | "service" | "loan" | "budget" | "category" | "reserve" | "account" => kind,
                _ => return Err(ToolFailure::Invalid("kind tiene que ser movement, card-purchase, savings, card, service, loan, budget, category, reserve o account.".into())),
            };
            if id.is_none() {
                return Err(ToolFailure::Invalid("Falta el id de lo que se borra: leelo antes con las herramientas de lectura.".into()));
            }
            (kind, "delete")
        }
        _ => return Err(ToolFailure::Invalid("La herramienta de Finanzas no existe.".into())),
    };
    // Editing a movement goes through the form of what it is.
    let kind = if action == "save" && id.is_some() && ["expense", "income", "salary"].contains(&kind.as_str()) {
        let stored = model::load_movement(connection, id.as_deref().unwrap_or_default())?.ok_or_else(|| ToolFailure::Invalid("Ese movimiento no existe.".into()))?;
        let is_payment = stored.card_purchase_id.is_some() || stored.loan_id.is_some() || stored.service_id.is_some();
        if is_payment { "movement".to_string() } else { kind }
    } else {
        kind
    };
    Ok((kind, id, action.to_string(), values))
}

/// `{purchase, number}` of the arguments as `purchaseId:number`, finding the
/// purchase by id or name.
fn installment_refs(connection: &Connection, chosen: &[Value]) -> ToolResult<Vec<Value>> {
    let purchases = model::list_purchases(connection)?;
    chosen
        .iter()
        .map(|item| {
            let purchase = text(item, "purchase").ok_or_else(|| ToolFailure::Invalid("Cada cuota necesita purchase (la compra) y number.".into()))?;
            let number = item.get("number").and_then(Value::as_u64).ok_or_else(|| ToolFailure::Invalid("Cada cuota necesita number.".into()))?;
            let found = purchases
                .iter()
                .find(|candidate| candidate.id == purchase)
                .or_else(|| purchases.iter().find(|candidate| fold(&candidate.name) == fold(&purchase)))
                .ok_or_else(|| ToolFailure::Invalid(format!("No encontré la compra «{purchase}».")))?;
            Ok(Value::String(format!("{}:{number}", found.id)))
        })
        .collect()
}

fn apply_in(connection: &Connection, source: &str, name: &str, arguments: &Value) -> ToolResult<ops::Applied> {
    let (kind, id, action, values) = request(connection, name, arguments)?;
    ops::apply(connection, source, &kind, id.as_deref(), &action, &values).map_err(|error| match error {
        OpError::Rejected { message, errors } => rejection(message, errors),
        OpError::Storage(message) => ToolFailure::Unavailable(message),
    })
}

fn source(context: &FinanceContext) -> &str {
    if context.source == "telegram" { "telegram" } else { "app" }
}

/// What the write would do, for the confirmation; nothing is kept.
pub(crate) fn preview(app: &crate::host::AppHandle, context: &FinanceContext, name: &str, arguments: &Value) -> ToolResult<String> {
    let mut connection = crate::finance::validate_context(context, app).map_err(ToolFailure::Unavailable)?;
    let transaction = connection.transaction()?;
    let applied = apply_in(&transaction, source(context), name, arguments)?;
    transaction.rollback()?;
    Ok(applied.message)
}

pub(crate) fn execute(app: &crate::host::AppHandle, context: &FinanceContext, name: &str, arguments: &Value) -> ToolResult<Value> {
    if is_finance_write_tool(name) {
        let applied = write(app, context, |connection| {
            apply_in(connection, source(context), name, arguments).map_err(|failure| match failure {
                ToolFailure::Invalid(message) => crate::finance::FinanceCommandError::invalid(message),
                ToolFailure::Unavailable(message) => crate::finance::FinanceCommandError::storage(message),
            })
        });
        return match applied {
            Ok(applied) => Ok(json!({ "ok": true, "changed": true, "message": applied.message, "savedId": applied.saved_id })),
            Err(error) if error.code != "storage" => Ok(json!({ "ok": false, "changed": false, "error": error.message })),
            Err(error) => Err(ToolFailure::Unavailable(error.message)),
        };
    }
    match name {
        "get_finance_dollar_quotes" => external(crate::host::async_runtime::block_on(crate::services::finance_external::dollar_quotes())),
        "get_finance_inflation_indices" => external(crate::host::async_runtime::block_on(crate::services::finance_external::inflation_indices())),
        "get_finance_historical_dollar_quotes" => {
            let (from, to) = (text(arguments, "from"), text(arguments, "to"));
            external(crate::host::async_runtime::block_on(crate::services::finance_external::historical_dollar_quotes(from.as_deref(), to.as_deref())))
        }
        _ => {
            let connection = crate::finance::validate_context(context, app).map_err(ToolFailure::Unavailable)?;
            read_tool(&connection, name, arguments)
        }
    }
}

fn external<T: serde::Serialize>(result: Result<T, String>) -> ToolResult<Value> {
    let value = result.map_err(ToolFailure::Unavailable)?;
    serde_json::to_value(value).map_err(|error| ToolFailure::Unavailable(error.to_string()))
}

fn month_argument(arguments: &Value) -> ToolResult<String> {
    match text(arguments, "month") {
        None => Ok(current_month()),
        Some(month) if valid_month(&month) => Ok(month),
        Some(_) => Err(ToolFailure::Invalid("month tiene que ser YYYY-MM.".into())),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> ToolResult<Value> {
    serde_json::to_value(value).map_err(|error| ToolFailure::Unavailable(error.to_string()))
}

fn read_tool(connection: &Connection, name: &str, arguments: &Value) -> ToolResult<Value> {
    match name {
        "get_finance_month" => to_json(&screen::overview(connection, &month_argument(arguments)?)?),
        "get_finance_payments" => to_json(&screen::payments(connection, &month_argument(arguments)?)?),
        "list_finance_movements" => list_movements(connection, arguments),
        "get_finance_setup" => setup(connection),
        _ => Err(ToolFailure::Invalid("La herramienta de Finanzas no existe.".into())),
    }
}

fn list_movements(connection: &Connection, arguments: &Value) -> ToolResult<Value> {
    let mut filter = MovementFilter {
        kind: text(arguments, "kind").unwrap_or_default(),
        search: text(arguments, "search").unwrap_or_default(),
        ..MovementFilter::default()
    };
    if let Some(category) = text(arguments, "category") {
        let found = model::list_categories(connection)?
            .into_iter()
            .find(|candidate| candidate.id == category || fold(&candidate.name) == fold(&category))
            .ok_or_else(|| ToolFailure::Invalid(format!("No encontré la categoría «{category}».")))?;
        filter.category_id = found.id;
    }
    let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 200) as usize;
    match text(arguments, "month") {
        Some(month) => {
            if !valid_month(&month) {
                return Err(ToolFailure::Invalid("month tiene que ser YYYY-MM.".into()));
            }
            let view = screen::movements_view(connection, &month, &filter)?;
            Ok(json!({
                "month": month,
                "count": view.rows.len(),
                "income": view.income,
                "expenses": view.expenses,
                "rows": view.rows.iter().take(limit).collect::<Vec<_>>(),
                "hasMore": view.rows.len() > limit,
            }))
        }
        None => {
            let result = screen::search(connection, &filter, 0)?;
            let rows = result.months.iter().flat_map(|month| month.rows.iter()).take(limit).collect::<Vec<_>>();
            Ok(json!({ "count": result.count, "income": result.income, "expenses": result.expenses, "rows": rows, "hasMore": result.count > limit }))
        }
    }
}

fn setup(connection: &Connection) -> ToolResult<Value> {
    let month = current_month();
    let categories = model::list_categories(connection)?;
    let balances = model::reserve_balances(connection)?;
    let purchases = model::purchase_states(connection)?;
    let names = |kind: &str| categories.iter().filter(|category| category.kind == kind).map(|category| category.name.clone()).collect::<Vec<_>>();
    let cards = model::list_cards(connection)?
        .into_iter()
        .map(|card| {
            let open = purchases
                .iter()
                .filter(|state| state.purchase.card_id == card.id && !state.finished)
                .map(|state| json!({
                    "id": state.purchase.id,
                    "name": state.purchase.name,
                    "total": state.purchase.total,
                    "currency": state.purchase.currency,
                    "installments": state.purchase.installment_count,
                    "paid": state.paid_count,
                    "remaining": state.remaining,
                }))
                .collect::<Vec<_>>();
            json!({ "id": card.id, "name": card.name, "purchasesInCourse": open })
        })
        .collect::<Vec<_>>();
    let services = model::services_of_month(connection, &month)?
        .into_iter()
        .map(|service| json!({
            "id": service.service.id,
            "name": service.service.name,
            "currency": service.service.currency,
            "amount": service.service.amount,
            "lastAmount": service.estimate,
            "paidThisMonth": service.paid.is_some(),
        }))
        .collect::<Vec<_>>();
    let loans = model::loan_states(connection)?
        .into_iter()
        .map(|state| json!({
            "id": state.loan.id,
            "name": state.loan.name,
            "currency": state.loan.currency,
            "amount": state.loan.amount,
            "installments": state.loan.installment_count,
            "paid": state.paid_count,
            "totalPaid": state.total_paid,
            "lastPaid": state.last_paid,
            "nextNumber": state.next_number,
        }))
        .collect::<Vec<_>>();
    let budgets = crate::finance_budgets::budgets_of_month(connection, &month)?;
    let reserves = model::list_reserves(connection)?
        .into_iter()
        .map(|reserve| json!({
            "id": reserve.id,
            "name": reserve.name,
            "currency": reserve.currency,
            "balance": crate::finance::format_cents(balances.get(&reserve.id).copied().unwrap_or(0)),
            "goal": reserve.goal,
        }))
        .collect::<Vec<_>>();
    Ok(json!({
        "expenseCategories": names("expense"),
        "incomeCategories": names("income"),
        "accounts": model::list_accounts(connection)?.into_iter().map(|account| account.name).collect::<Vec<_>>(),
        "cards": cards,
        "services": services,
        "loans": loans,
        "budgets": budgets,
        "reserves": reserves,
        "spentThisMonth": screen::overview(connection, &month)?.balance.iter().map(|balance| json!({ "currency": balance.currency, "expenses": balance.expenses, "income": balance.income, "left": balance.left })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_tool_reaches_finanzas() {
        let connection = crate::finance::tests::database();
        connection.execute_batch("INSERT INTO finance_cards (id,name,created_at,updated_at) VALUES ('card','Visa','x','x');").expect("card");
        let preview = |name: &str, arguments: Value| apply_in(&connection, "app", name, &arguments).map(|applied| applied.message);
        assert!(preview("save_finance_expense", json!({ "name": "Nafta", "amount": "45000", "category": "transporte" })).expect("expense").contains("Nafta"));
        assert!(preview("save_finance_expense", json!({ "name": "Nafta", "amount": "45000", "category": "Inventada" })).is_err());
        assert!(preview("save_finance_salary", json!({ "amount": "1.500.000" })).expect("salary").contains("1.500.000"));
        let purchase = preview("save_finance_card_purchase", json!({ "card": "visa", "name": "Heladera", "total": "720000", "installments": 12 })).expect("purchase");
        assert!(purchase.contains("12 cuotas de $ 60.000"), "{purchase}");
        assert!(preview("save_finance_setup", json!({ "kind": "service", "name": "Luz", "amount": "18000" })).expect("service").contains("Luz"));
        assert!(matches!(preview("delete_finance_record", json!({ "kind": "movement" })), Err(ToolFailure::Invalid(_))));
    }
}
