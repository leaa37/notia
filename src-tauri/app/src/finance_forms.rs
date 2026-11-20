//! Formularios de Finanzas: qué se carga a mano y cómo se pide.
//!
//! Rust arma cada formulario (campos, opciones, valores por defecto, ayuda)
//! y la web y la app de Android solo lo dibujan. Se pide lo mínimo: lo
//! demás tiene un valor por defecto que no hace falta tocar (la fecha es
//! hoy, la moneda es pesos, la primera cuota es el mes que viene).
//!
//! Un campo con `reload` vuelve a pedir el formulario cuando cambia, con
//! los valores cargados y `_changed`: así el pago de un servicio trae el
//! importe del servicio elegido y el pago de una tarjeta, sus cuotas.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::finance::{
    add_months, amount_input, current_month, money_text, month_label, month_short, today, CATEGORY_OTHER, CATEGORY_OTHER_INCOME,
    CATEGORY_SALARY,
};
use crate::finance_model::{self as model, cents, Category, LoanState, Named, PurchaseState, Reserve, Service};
use crate::finance_ops::{text, Values};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn choice(value: impl Into<String>, label: impl Into<String>) -> Choice {
    Choice { value: value.into(), label: label.into(), detail: None }
}

/// The currency next to an amount: its own key in the values.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyPick {
    pub key: String,
    pub value: String,
    /// The amount is in this currency and it cannot change.
    pub locked: bool,
}

/// One field. `type` is `text`, `money`, `integer`, `date`, `month`,
/// `select`, `checks` (several options, the value is the list of chosen
/// ones) or `info` (a line to read).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: Value,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<CurrencyPick>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reload: bool,
}

impl Field {
    fn new(kind: &'static str, key: &str, label: &str) -> Self {
        Field {
            key: key.into(),
            label: label.into(),
            kind,
            value: Value::String(String::new()),
            required: false,
            help: None,
            placeholder: None,
            options: Vec::new(),
            suggestions: Vec::new(),
            currency: None,
            reload: false,
        }
    }
    fn text(key: &str, label: &str) -> Self {
        Self::new("text", key, label)
    }
    fn money(key: &str, label: &str, currency: &str) -> Self {
        Field { currency: Some(CurrencyPick { key: "currency".into(), value: currency.into(), locked: false }), ..Self::new("money", key, label) }
    }
    fn locked_money(key: &str, label: &str, currency: &str) -> Self {
        Field { currency: Some(CurrencyPick { key: String::new(), value: currency.into(), locked: true }), ..Self::new("money", key, label) }
    }
    fn integer(key: &str, label: &str) -> Self {
        Self::new("integer", key, label)
    }
    fn date(key: &str, label: &str) -> Self {
        Self::new("date", key, label)
    }
    fn month(key: &str, label: &str) -> Self {
        Self::new("month", key, label)
    }
    fn select(key: &str, label: &str, options: Vec<Choice>) -> Self {
        Field { options, ..Self::new("select", key, label) }
    }
    fn checks(key: &str, label: &str, options: Vec<Choice>) -> Self {
        let chosen = options.iter().map(|option| Value::String(option.value.clone())).collect();
        Field { options, value: Value::Array(chosen), ..Self::new("checks", key, label) }
    }
    fn info(key: &str, label: &str, text: impl Into<String>) -> Self {
        Field { value: Value::String(text.into()), ..Self::new("info", key, label) }
    }
    fn required(mut self) -> Self {
        self.required = true;
        self
    }
    fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
    fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }
    fn suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }
    fn default(mut self, value: impl Into<Value>) -> Self {
        self.value = value.into();
        self
    }
    fn reload(mut self) -> Self {
        self.reload = true;
        self
    }
    fn currency_value(mut self, value: &str) -> Self {
        if let Some(currency) = &mut self.currency {
            currency.value = value.into();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormDelete {
    pub label: String,
    pub confirm: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryForm {
    pub kind: String,
    pub id: Option<String>,
    pub title: String,
    pub intro: Option<String>,
    pub submit: String,
    pub fields: Vec<Field>,
    pub delete: Option<FormDelete>,
}

fn delete(label: &str, confirm: &str) -> Option<FormDelete> {
    Some(FormDelete { label: label.into(), confirm: confirm.into() })
}

// ---------------------------------------------------------------------------
// The menu
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuItem {
    pub kind: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuGroup {
    pub title: &'static str,
    pub items: Vec<MenuItem>,
}

fn item(kind: &'static str, label: &'static str, description: &'static str) -> MenuItem {
    MenuItem { kind, label, description }
}

pub(crate) fn menu() -> Vec<MenuGroup> {
    vec![
        MenuGroup {
            title: "Del día a día",
            items: vec![
                item("expense", "Gasto", "Nombre, importe y categoría"),
                item("card-purchase", "Compra con tarjeta", "En una o en varias cuotas"),
                item("income", "Ingreso", "Lo que entra aparte del sueldo"),
                item("salary", "Sueldo", "Lo que cobraste este mes"),
            ],
        },
        MenuGroup {
            title: "Pagos del mes",
            items: vec![
                item("service-payment", "Pagar un servicio", "Luz, internet, alquiler…"),
                item("card-payment", "Pagar la tarjeta", "Las cuotas que vencen"),
                item("loan-payment", "Pagar una cuota de préstamo", "Con lo que pagaste"),
            ],
        },
        MenuGroup {
            title: "Ahorro",
            items: vec![
                item("savings", "Guardar o sacar ahorro", "En pesos o en dólares"),
                item("exchange", "Comprar o vender dólares", "Con los pesos que pagaste"),
            ],
        },
        MenuGroup {
            title: "Configurar",
            items: vec![
                item("card", "Tarjeta", "Para cargarle compras"),
                item("service", "Servicio", "Lo que pagás todos los meses"),
                item("loan", "Préstamo", "Monto y cuotas"),
                item("budget", "Presupuesto", "Un tope por categoría"),
                item("category", "Categoría", "De gastos o de ingresos"),
                item("reserve", "Reserva de ahorro", "Con un objetivo, si querés"),
                item("account", "Cuenta", "Para anotar de dónde sale la plata"),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// What the forms offer
// ---------------------------------------------------------------------------

/// Everything a form may offer to choose from.
pub(crate) struct Catalog {
    pub categories: Vec<Category>,
    pub accounts: Vec<Named>,
    pub cards: Vec<Named>,
    pub services: Vec<Service>,
    pub loans: Vec<LoanState>,
    pub purchases: Vec<PurchaseState>,
    pub reserves: Vec<Reserve>,
    /// Names of recent expenses, to suggest while typing.
    pub recent_names: Vec<String>,
}

pub(crate) fn catalog(connection: &Connection) -> rusqlite::Result<Catalog> {
    let mut statement = connection.prepare(
        "SELECT name FROM finance_movements WHERE kind='expense' AND card_purchase_id IS NULL AND loan_id IS NULL AND service_id IS NULL
         GROUP BY name ORDER BY MAX(date) DESC LIMIT 40",
    )?;
    let recent_names = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(Catalog {
        categories: model::list_categories(connection)?,
        accounts: model::list_accounts(connection)?,
        cards: model::list_cards(connection)?,
        services: model::list_services(connection)?,
        loans: model::loan_states(connection)?,
        purchases: model::purchase_states(connection)?,
        reserves: model::list_reserves(connection)?,
        recent_names,
    })
}

impl Catalog {
    fn categories(&self, kind: &str) -> Vec<Choice> {
        self.categories.iter().filter(|category| category.kind == kind).map(|category| choice(&category.id, &category.name)).collect()
    }

    fn named(list: &[Named]) -> Vec<Choice> {
        list.iter().map(|item| choice(&item.id, &item.name)).collect()
    }

    fn reserves(&self, currency: Option<&str>) -> Vec<Choice> {
        self.reserves
            .iter()
            .filter(|reserve| currency.is_none_or(|currency| reserve.currency == currency))
            .map(|reserve| choice(&reserve.id, format!("{} ({})", reserve.name, if reserve.currency == "USD" { "US$" } else { "$" })))
            .collect()
    }
}

/// The optional account of a movement, only when there are accounts.
fn account_field(catalog: &Catalog) -> Option<Field> {
    if catalog.accounts.is_empty() {
        return None;
    }
    let mut options = vec![choice("", "Sin cuenta")];
    options.extend(Catalog::named(&catalog.accounts));
    Some(Field::select("accountId", "Cuenta", options))
}

fn date_field() -> Field {
    Field::date("date", "Fecha").default(today())
}

// ---------------------------------------------------------------------------
// Building a form
// ---------------------------------------------------------------------------

/// The form of `kind`: blank (with `prefill`), or `id` to edit. `prefill`
/// also carries what is typed when a field with `reload` changes, with
/// `_changed` set to that field's key.
pub(crate) fn build(connection: &Connection, catalog: &Catalog, kind: &str, id: Option<&str>, prefill: &Values) -> rusqlite::Result<Option<EntryForm>> {
    let stored = match id {
        Some(id) => match stored_values(connection, catalog, kind, id)? {
            Some(stored) => Some(stored),
            None => return Ok(None),
        },
        None => None,
    };
    let (kind, stored_values) = match &stored {
        Some((kind, values)) => (kind.as_str(), values.clone()),
        None => (kind, Map::new()),
    };
    let changed = text(prefill, "_changed");
    // What depends on the field that changed belongs to the previous choice.
    let stale: &[&str] = match changed.as_deref() {
        Some("serviceId") => &["amount", "currency"],
        Some("loanId") => &["amount", "number"],
        Some("cardId") | Some("month") if kind == "card-payment" => &["installments"],
        _ => &[],
    };
    let mut values = stored_values;
    for (key, value) in prefill {
        if key != "_changed" && !stale.contains(&key.as_str()) {
            values.insert(key.clone(), value.clone());
        }
    }
    let editing = id.is_some();
    let form = match kind {
        "expense" => Some(expense_form(catalog, editing)),
        "income" => Some(income_form(catalog, editing)),
        "salary" => Some(salary_form(connection, catalog, editing)?),
        "payment" => Some(payment_form(connection, catalog, &values)?),
        "card-purchase" => Some(purchase_form(catalog, &values, editing)),
        "card-payment" => Some(card_payment_form(catalog, &values)),
        "loan-payment" => Some(loan_payment_form(catalog, &values)),
        "service-payment" => Some(service_payment_form(connection, catalog, &values)?),
        "savings" => Some(savings_form(catalog, &values)),
        "exchange" => Some(exchange_form(catalog, &values)),
        "category" => Some(category_form(editing)),
        "account" => Some(named_form("account", "Cuenta", "Banco, efectivo, billetera…", editing)),
        "card" => Some(named_form("card", "Tarjeta", "Visa, Mastercard…", editing)),
        "service" => Some(service_form(editing)),
        "loan" => Some(loan_form(&values, editing)),
        "budget" => Some(budget_form(catalog, editing)),
        "reserve" => Some(reserve_form(editing)),
        _ => None,
    };
    Ok(form.map(|mut form| {
        fill(&mut form.fields, &values);
        form.id = id.map(str::to_string);
        if !editing {
            form.delete = None;
        } else if form.kind == "payment" {
            form.kind = "movement".into();
        }
        form
    }))
}

/// Puts each value in its field (and the currency in its pick).
fn fill(fields: &mut [Field], values: &Values) {
    for field in fields {
        if field.kind == "info" {
            continue;
        }
        if let Some(value) = values.get(&field.key).filter(|value| !value.is_null()) {
            field.value = match (field.kind, value) {
                ("money", Value::String(text)) => Value::String(money_text_input(text)),
                ("integer", Value::Number(number)) => Value::String(number.to_string()),
                _ => value.clone(),
            };
        }
        if field.kind == "money" {
            if let Value::String(text) = &field.value {
                field.value = Value::String(money_text_input(text));
            }
        }
        if let Some(pick) = &mut field.currency {
            if !pick.locked {
                if let Some(currency) = values.get(&pick.key).and_then(Value::as_str).filter(|currency| crate::finance::valid_currency(currency)) {
                    pick.value = currency.to_string();
                }
            }
        }
    }
}

/// A stored amount (`1500.50`) as it is typed (`1500,50`); what is being
/// typed stays as it is.
fn money_text_input(value: &str) -> String {
    let stored = value.split_once('.').is_some_and(|(whole, fraction)| {
        !whole.is_empty() && whole.chars().all(|c| c.is_ascii_digit()) && fraction.len() == 2 && fraction.chars().all(|c| c.is_ascii_digit())
    });
    if stored {
        amount_input(cents(value))
    } else {
        value.to_string()
    }
}

/// The values of a stored record, and the form that edits it.
fn stored_values(connection: &Connection, catalog: &Catalog, kind: &str, id: &str) -> rusqlite::Result<Option<(String, Values)>> {
    let object = |value: Value| value.as_object().cloned().unwrap_or_default();
    Ok(match kind {
        "movement" | "expense" | "income" | "salary" => model::load_movement(connection, id)?.map(|movement| {
            let form = if movement.card_purchase_id.is_some() || movement.loan_id.is_some() || movement.service_id.is_some() {
                "payment"
            } else if movement.kind == "income" && movement.category_id.as_deref() == Some(CATEGORY_SALARY.0) {
                "salary"
            } else if movement.kind == "income" {
                "income"
            } else {
                "expense"
            };
            let values = object(json!({
                "name": movement.name,
                "amount": movement.amount,
                "currency": movement.currency,
                "categoryId": movement.category_id.unwrap_or_default(),
                "accountId": movement.account_id.unwrap_or_default(),
                "date": movement.date,
                "_movement": movement.id,
            }));
            (form.to_string(), values)
        }),
        "card-purchase" => catalog.purchases.iter().find(|state| state.purchase.id == id).map(|state| {
            let purchase = &state.purchase;
            ("card-purchase".into(), object(json!({
                "cardId": purchase.card_id,
                "name": purchase.name,
                "total": purchase.total,
                "currency": purchase.currency,
                "installmentCount": purchase.installment_count,
                "categoryId": purchase.category_id.clone().unwrap_or_default(),
                "firstMonth": purchase.first_month,
                "_paid": state.paid_count,
            })))
        }),
        "savings" | "exchange" => model::load_savings_movement(connection, id)?.map(|movement| {
            let exchange = movement.pesos.is_some();
            let values = if exchange {
                object(json!({
                    "type": if movement.kind == "withdrawal" { "sell" } else { "buy" },
                    "dollars": movement.amount,
                    "pesos": movement.pesos,
                    "reserveId": movement.reserve_id,
                    "date": movement.date,
                }))
            } else {
                object(json!({
                    "type": movement.kind,
                    "reserveId": movement.reserve_id,
                    "amount": movement.amount,
                    "date": movement.date,
                    "note": movement.note,
                }))
            };
            ((if exchange { "exchange" } else { "savings" }).to_string(), values)
        }),
        "category" => catalog.categories.iter().find(|category| category.id == id).map(|category| {
            ("category".into(), object(json!({ "name": category.name, "kind": category.kind })))
        }),
        "account" => catalog.accounts.iter().find(|account| account.id == id).map(|account| ("account".into(), object(json!({ "name": account.name })))),
        "card" => catalog.cards.iter().find(|card| card.id == id).map(|card| ("card".into(), object(json!({ "name": card.name })))),
        "service" => catalog.services.iter().find(|service| service.id == id).map(|service| ("service".into(), object(json!({ "name": service.name })))),
        "loan" => catalog.loans.iter().find(|state| state.loan.id == id).map(|state| {
            let loan = &state.loan;
            ("loan".into(), object(json!({
                "name": loan.name,
                "amount": loan.amount,
                "currency": loan.currency,
                "installmentCount": loan.installment_count,
                "firstMonth": loan.first_month,
                "_paid": state.paid_count,
                "_totalPaid": state.total_paid,
            })))
        }),
        "budget" => model::list_budgets(connection)?.into_iter().find(|budget| budget.id == id).map(|budget| {
            ("budget".into(), object(json!({ "categoryId": budget.category_id, "amount": budget.amount, "currency": budget.currency })))
        }),
        "reserve" => catalog.reserves.iter().find(|reserve| reserve.id == id).map(|reserve| {
            ("reserve".into(), object(json!({ "name": reserve.name, "currency": reserve.currency, "goal": reserve.goal.clone().unwrap_or_default() })))
        }),
        _ => None,
    })
}

fn form(kind: &str, title: &str, submit: &str, fields: Vec<Field>) -> EntryForm {
    EntryForm { kind: kind.into(), id: None, title: title.into(), intro: None, submit: submit.into(), fields, delete: None }
}

fn with_intro(mut form: EntryForm, intro: impl Into<String>) -> EntryForm {
    form.intro = Some(intro.into());
    form
}

fn push_account(fields: &mut Vec<Field>, catalog: &Catalog) {
    if let Some(field) = account_field(catalog) {
        fields.push(field);
    }
}

fn expense_form(catalog: &Catalog, editing: bool) -> EntryForm {
    let mut fields = vec![
        Field::text("name", "¿En qué?").required().placeholder("Nafta, almuerzo, farmacia…").suggestions(catalog.recent_names.clone()),
        Field::money("amount", "Importe", "ARS").required(),
        Field::select("categoryId", "Categoría", catalog.categories("expense")).required(),
        date_field(),
    ];
    push_account(&mut fields, catalog);
    let mut form = form("expense", if editing { "Gasto" } else { "Nuevo gasto" }, "Guardar", fields);
    form.delete = delete("Borrar gasto", "¿Borrar este gasto?");
    form
}

fn income_form(catalog: &Catalog, editing: bool) -> EntryForm {
    let mut fields = vec![
        Field::text("name", "¿De qué?").required().placeholder("Venta, regalo, changa…"),
        Field::money("amount", "Importe", "ARS").required(),
        Field::select("categoryId", "Categoría", catalog.categories("income")).default(CATEGORY_OTHER_INCOME.0),
        date_field(),
    ];
    push_account(&mut fields, catalog);
    let mut form = form("income", if editing { "Ingreso" } else { "Nuevo ingreso" }, "Guardar", fields);
    form.delete = delete("Borrar ingreso", "¿Borrar este ingreso?");
    form
}

fn salary_form(connection: &Connection, catalog: &Catalog, editing: bool) -> rusqlite::Result<EntryForm> {
    let last: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT amount,currency,account_id FROM finance_movements WHERE kind='income' AND category_id=?1 ORDER BY date DESC, created_at DESC LIMIT 1",
            [CATEGORY_SALARY.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let mut amount = Field::money("amount", "Lo que cobraste", "ARS").required();
    let mut fields = Vec::new();
    if let (Some((last_amount, currency, account)), false) = (&last, editing) {
        amount = amount.default(last_amount.clone()).currency_value(currency).help(format!("El último fue de {}.", money_text(cents(last_amount), currency)));
        if let Some(account) = account {
            fields.push(("accountId", account.clone()));
        }
    }
    let mut list = vec![amount, date_field()];
    push_account(&mut list, catalog);
    for (key, value) in fields {
        if let Some(field) = list.iter_mut().find(|field| field.key == key) {
            field.value = Value::String(value);
        }
    }
    let mut form = with_intro(form("salary", "Sueldo", "Guardar", list), "En mano, lo que entró a la cuenta.");
    form.delete = delete("Borrar sueldo", "¿Borrar este sueldo?");
    Ok(form)
}

/// A payment already made: what it pays stays, the rest can change.
fn payment_form(connection: &Connection, catalog: &Catalog, values: &Values) -> rusqlite::Result<EntryForm> {
    let id = text(values, "_movement").unwrap_or_default();
    let movement = model::load_movement(connection, &id)?;
    let mut what = String::new();
    let mut card_payment = false;
    if let Some(movement) = &movement {
        if let Some(purchase) = movement.card_purchase_id.as_deref().and_then(|id| catalog.purchases.iter().find(|state| state.purchase.id == id)) {
            card_payment = true;
            let card = catalog.cards.iter().find(|card| card.id == purchase.purchase.card_id).map(|card| card.name.clone()).unwrap_or_default();
            what = format!("Cuota {}/{} de {} · {card}", movement.installment_number.unwrap_or(1), purchase.purchase.installment_count, purchase.purchase.name);
        } else if let Some(loan) = movement.loan_id.as_deref().and_then(|id| catalog.loans.iter().find(|state| state.loan.id == id)) {
            what = format!("Cuota {}/{} de {}", movement.installment_number.unwrap_or(1), loan.loan.installment_count, loan.loan.name);
        } else if let Some(service) = movement.service_id.as_deref().and_then(|id| catalog.services.iter().find(|service| service.id == id)) {
            what = format!("{} de {}", service.name, movement.service_month.as_deref().map(month_label).unwrap_or_default());
        }
    }
    let currency = movement.as_ref().map(|movement| movement.currency.clone()).unwrap_or_else(|| "ARS".into());
    let mut fields = vec![
        Field::info("what", "Pago de", what),
        Field::text("name", "Nombre").required(),
        Field::locked_money("amount", "Importe", &currency).required(),
    ];
    if card_payment {
        fields.push(Field::select("categoryId", "Categoría", catalog.categories("expense")));
    }
    fields.push(date_field());
    push_account(&mut fields, catalog);
    let mut form = form("payment", "Pago", "Guardar", fields);
    form.delete = delete("Deshacer el pago", "¿Deshacer este pago? Vuelve a quedar pendiente.");
    Ok(form)
}

fn purchase_form(catalog: &Catalog, values: &Values, editing: bool) -> EntryForm {
    let mut fields = Vec::new();
    if catalog.cards.is_empty() {
        fields.push(Field::text("newCard", "Tarjeta").required().placeholder("Visa, Mastercard…").help("Todavía no tenés tarjetas: se crea con este nombre."));
    } else {
        let mut card = Field::select("cardId", "Tarjeta", Catalog::named(&catalog.cards)).required();
        if catalog.cards.len() == 1 {
            card = card.default(catalog.cards[0].id.clone());
        }
        fields.push(card);
    }
    fields.extend([
        Field::text("name", "¿Qué compraste?").required().placeholder("Heladera, zapatillas…"),
        Field::money("total", "Total, con intereses", "ARS").required(),
        Field::integer("installmentCount", "Cuotas").default("1").required(),
        Field::select("categoryId", "Categoría", catalog.categories("expense")).default(CATEGORY_OTHER.0),
        Field::month("firstMonth", "Primera cuota").default(add_months(&current_month(), 1)).help("El mes en que la pagás con el resumen."),
    ]);
    let paid = values.get("_paid").and_then(Value::as_u64).unwrap_or(0);
    let mut form = form("card-purchase", if editing { "Compra con tarjeta" } else { "Nueva compra con tarjeta" }, "Guardar", fields);
    if editing && paid > 0 {
        form.intro = Some(format!("Ya pagaste {paid} {}.", if paid == 1 { "cuota" } else { "cuotas" }));
    }
    form.delete = delete("Borrar compra", "¿Borrar la compra? Se borran las cuotas que faltan; las ya pagadas quedan en Movimientos.");
    form
}

fn card_payment_form(catalog: &Catalog, values: &Values) -> EntryForm {
    let card_id = text(values, "cardId").or_else(|| catalog.cards.first().map(|card| card.id.clone())).unwrap_or_default();
    let month = text(values, "month").filter(|month| crate::finance::valid_month(month)).unwrap_or_else(current_month);
    let due = model::card_installments(&catalog.purchases, &card_id, &month).into_iter().filter(|installment| installment.paid.is_none()).collect::<Vec<_>>();
    let options = due
        .iter()
        .map(|installment| {
            let label = if installment.count == 1 { installment.name.clone() } else { format!("{} · cuota {}/{}", installment.name, installment.number, installment.count) };
            let late = if installment.late { format!(" · de {}", month_short(&installment.month)) } else { String::new() };
            Choice {
                value: format!("{}:{}", installment.purchase_id, installment.number),
                label,
                detail: Some(format!("{}{late}", money_text(cents(&installment.amount), &installment.currency))),
            }
        })
        .collect::<Vec<_>>();
    let mut fields = vec![
        Field::select("cardId", "Tarjeta", Catalog::named(&catalog.cards)).required().default(card_id.clone()).reload(),
        Field::month("month", "Resumen de").default(month.clone()).reload(),
    ];
    if options.is_empty() {
        fields.push(Field::info("installments", "Cuotas", format!("No hay cuotas para pagar hasta {}.", month_label(&month))));
    } else {
        let mut totals = std::collections::BTreeMap::<String, i128>::new();
        for installment in &due {
            *totals.entry(installment.currency.clone()).or_default() += cents(&installment.amount);
        }
        let total = totals.iter().map(|(currency, amount)| money_text(*amount, currency)).collect::<Vec<_>>().join(" + ");
        fields.push(Field::checks("installments", "Cuotas a pagar", options).required().help(format!("Todas: {total}.")));
    }
    fields.push(date_field());
    push_account(&mut fields, catalog);
    let form = form("card-payment", "Pagar la tarjeta", "Pagar", fields);
    if catalog.cards.is_empty() {
        return with_intro(form, "Todavía no tenés tarjetas. Creá una desde Cargar → Tarjeta.");
    }
    form
}

fn loan_payment_form(catalog: &Catalog, values: &Values) -> EntryForm {
    let open = catalog.loans.iter().filter(|state| !state.finished).collect::<Vec<_>>();
    let loan_id = text(values, "loanId").or_else(|| open.first().map(|state| state.loan.id.clone())).unwrap_or_default();
    let state = catalog.loans.iter().find(|state| state.loan.id == loan_id);
    let mut fields = vec![Field::select("loanId", "Préstamo", open.iter().map(|state| choice(&state.loan.id, &state.loan.name)).collect())
        .required()
        .default(loan_id.clone())
        .reload()];
    if let Some(state) = state {
        let unpaid = state
            .installments
            .iter()
            .filter(|installment| installment.paid.is_none())
            .map(|installment| choice(installment.number.to_string(), format!("Cuota {} de {} · {}", installment.number, state.loan.installment_count, month_short(&installment.month))))
            .collect::<Vec<_>>();
        let number = Field::select("number", "Cuota", unpaid).required().default(state.next_number.map(|number| number.to_string()).unwrap_or_default());
        // Each installment costs something else: typed every time, the last one only as a reminder.
        let mut amount = Field::locked_money("amount", "Lo que pagaste", &state.loan.currency).required();
        if let Some(last) = &state.last_paid {
            amount = amount.help(format!("La anterior fue de {}.", money_text(cents(last), &state.loan.currency)));
        }
        fields.push(number);
        fields.push(amount);
    }
    fields.push(date_field());
    push_account(&mut fields, catalog);
    let form = form("loan-payment", "Pagar una cuota", "Pagar", fields);
    if open.is_empty() {
        return with_intro(form, "No tenés préstamos con cuotas por pagar. Creá uno desde Cargar → Préstamo.");
    }
    form
}

fn service_payment_form(connection: &Connection, catalog: &Catalog, values: &Values) -> rusqlite::Result<EntryForm> {
    let service_id = text(values, "serviceId").or_else(|| catalog.services.first().map(|service| service.id.clone())).unwrap_or_default();
    let mut fields = vec![
        Field::select("serviceId", "Servicio", catalog.services.iter().map(|service| choice(&service.id, &service.name)).collect())
            .required()
            .default(service_id.clone())
            .reload(),
        Field::month("month", "Mes").default(current_month()),
    ];
    // What this month cost, typed every time: the last one is only a reminder.
    let last = model::last_service_payment(connection, &service_id)?;
    let currency = last.as_ref().map(|(_, currency)| currency.as_str()).unwrap_or("ARS");
    let mut amount = Field::money("amount", "Lo que pagaste este mes", currency).required();
    if let Some((paid, currency)) = &last {
        amount = amount.help(format!("La vez anterior pagaste {}.", money_text(cents(paid), currency)));
    }
    fields.push(amount);
    fields.push(date_field());
    push_account(&mut fields, catalog);
    let form = form("service-payment", "Pagar un servicio", "Pagar", fields);
    if catalog.services.is_empty() {
        return Ok(with_intro(form, "Todavía no tenés servicios. Creá uno desde Cargar → Servicio."));
    }
    Ok(form)
}

fn savings_form(catalog: &Catalog, values: &Values) -> EntryForm {
    let reserve_id = text(values, "reserveId").or_else(|| catalog.reserves.first().map(|reserve| reserve.id.clone())).unwrap_or_default();
    let currency = catalog.reserves.iter().find(|reserve| reserve.id == reserve_id).map(|reserve| reserve.currency.as_str()).unwrap_or("ARS");
    let fields = vec![
        Field::select("type", "", vec![choice("deposit", "Guardar"), choice("withdrawal", "Sacar")]).default("deposit").required(),
        Field::select("reserveId", "Reserva", catalog.reserves(None)).required().default(reserve_id.clone()).reload(),
        Field::locked_money("amount", "Importe", currency).required(),
        date_field(),
        Field::text("note", "Nota").placeholder("Opcional"),
    ];
    let mut form = form("savings", "Ahorro", "Guardar", fields);
    form.delete = delete("Borrar movimiento", "¿Borrar este movimiento de ahorro?");
    form
}

fn exchange_form(catalog: &Catalog, values: &Values) -> EntryForm {
    let selling = text(values, "type").as_deref() == Some("sell");
    let fields = vec![
        Field::select("type", "", vec![choice("buy", "Comprar dólares"), choice("sell", "Vender dólares")]).default("buy").required().reload(),
        Field::locked_money("dollars", "Dólares", "USD").required(),
        Field::locked_money("pesos", if selling { "Pesos que recibiste" } else { "Pesos que pagaste" }, "ARS").required(),
        Field::select("reserveId", "Reserva", catalog.reserves(Some("USD"))).default(catalog.reserves.iter().find(|reserve| reserve.currency == "USD").map(|reserve| reserve.id.clone()).unwrap_or_default()),
        date_field(),
    ];
    let mut form = with_intro(form("exchange", "Dólares", "Guardar", fields), "Los pesos cuentan como ahorro del mes; los dólares quedan en la reserva.");
    form.delete = delete("Borrar", "¿Borrar esta compra o venta de dólares?");
    form
}

fn category_form(editing: bool) -> EntryForm {
    let mut form = form(
        "category",
        if editing { "Categoría" } else { "Nueva categoría" },
        "Guardar",
        vec![
            Field::text("name", "Nombre").required(),
            Field::select("kind", "Es de", vec![choice("expense", "Gastos"), choice("income", "Ingresos")]).default("expense").required(),
        ],
    );
    form.delete = delete("Borrar categoría", "¿Borrar la categoría? Sus movimientos quedan sin categoría y se borra su presupuesto.");
    form
}

fn named_form(kind: &str, title: &str, placeholder: &str, editing: bool) -> EntryForm {
    let mut form = form(kind, &if editing { title.to_string() } else { format!("Nueva {}", title.to_lowercase()) }, "Guardar", vec![
        Field::text("name", "Nombre").required().placeholder(placeholder),
    ]);
    form.delete = match kind {
        "card" => delete("Borrar tarjeta", "¿Borrar la tarjeta y sus compras? Las cuotas que faltan desaparecen; las ya pagadas quedan en Movimientos."),
        _ => delete("Borrar cuenta", "¿Borrar la cuenta? Sus movimientos quedan, sin cuenta."),
    };
    form
}

fn service_form(editing: bool) -> EntryForm {
    let mut form = with_intro(
        form("service", if editing { "Servicio" } else { "Nuevo servicio" }, "Guardar", vec![
            Field::text("name", "Nombre").required().placeholder("Luz, internet, alquiler…"),
        ]),
        "Cada mes, al pagarlo, cargás lo que costó ese mes. Va a la categoría Servicios.",
    );
    form.delete = delete("Borrar servicio", "¿Borrar el servicio? Sus pagos quedan en Movimientos.");
    form
}

fn loan_form(values: &Values, editing: bool) -> EntryForm {
    let mut form = form("loan", if editing { "Préstamo" } else { "Nuevo préstamo" }, "Guardar", vec![
        Field::text("name", "Nombre").required().placeholder("Préstamo del banco…"),
        Field::money("amount", "Monto", "ARS").required().help("Lo que te prestaron."),
        Field::integer("installmentCount", "Cuotas").required(),
        Field::month("firstMonth", "Primera cuota").default(add_months(&current_month(), 1)),
    ]);
    if editing {
        let paid = values.get("_paid").and_then(Value::as_u64).unwrap_or(0);
        let currency = text(values, "currency").unwrap_or_else(|| "ARS".into());
        let total = text(values, "_totalPaid").map(|total| money_text(cents(&total), &currency)).unwrap_or_default();
        form.intro = Some(if paid == 0 { "Todavía no pagaste ninguna cuota.".into() } else { format!("Pagaste {paid} cuotas: {total}.") });
    }
    form.delete = delete("Borrar préstamo", "¿Borrar el préstamo? Las cuotas ya pagadas quedan en Movimientos.");
    form
}

fn budget_form(catalog: &Catalog, editing: bool) -> EntryForm {
    let mut form = with_intro(
        form("budget", if editing { "Presupuesto" } else { "Nuevo presupuesto" }, "Guardar", vec![
            Field::select("categoryId", "Categoría", catalog.categories("expense")).required(),
            Field::money("amount", "Tope por mes", "ARS").required(),
        ]),
        "Te aviso por Telegram al llegar al 80 % y al pasarte.",
    );
    form.delete = delete("Borrar presupuesto", "¿Borrar este presupuesto?");
    form
}

fn reserve_form(editing: bool) -> EntryForm {
    let mut form = form("reserve", if editing { "Reserva" } else { "Nueva reserva" }, "Guardar", vec![
        Field::text("name", "Nombre").required().placeholder("Vacaciones, fondo de emergencia…"),
        Field::money("goal", "Objetivo", "ARS").placeholder("Opcional"),
    ]);
    form.delete = delete("Borrar reserva", "¿Borrar la reserva y sus movimientos?");
    form
}
