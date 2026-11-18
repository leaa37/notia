//! Formularios de Finanzas: qué se carga a mano y cómo se pide.
//!
//! Rust arma cada formulario (campos, opciones, valores, ayuda) y la web y
//! la app de Android solo lo dibujan con un renderizador genérico. Las
//! claves de los campos son las de los argumentos de las herramientas del
//! asistente (`amount`, `accountId`, `effectiveDate`…), así la carga manual
//! pasa por los mismos constructores y los errores nombran el mismo campo.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::finance::{FinanceAccount, FinanceCategory, FinanceSavingsReserve, FinanceService};
use crate::finance_budgets::money_text;
use crate::finance_loans::LoanSummary;

pub(crate) type Values = Map<String, Value>;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
}

fn choice(value: impl Into<String>, label: impl Into<String>) -> Choice {
    Choice { value: value.into(), label: label.into() }
}

/// One field. `type` is `text`, `multiline`, `money`, `integer`, `date`,
/// `month`, `select`, `toggle` or `items` (rows with `columns`).
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<Field>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_label: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
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
            columns: Vec::new(),
            add_label: None,
            read_only: false,
        }
    }
    fn text(key: &str, label: &str) -> Self {
        Self::new("text", key, label)
    }
    fn multiline(key: &str, label: &str) -> Self {
        Self::new("multiline", key, label)
    }
    fn money(key: &str, label: &str) -> Self {
        Self::new("money", key, label)
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
    fn toggle(key: &str, label: &str) -> Self {
        Field { value: Value::Bool(false), ..Self::new("toggle", key, label) }
    }
    fn items(key: &str, label: &str, add_label: &str, columns: Vec<Field>) -> Self {
        Field { value: Value::Array(Vec::new()), columns, add_label: Some(add_label.into()), ..Self::new("items", key, label) }
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
    fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormDelete {
    pub label: String,
    pub confirm: String,
}

/// Another thing the form's record can do (for example, take back the
/// payment of an installment).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormAction {
    pub id: String,
    pub label: String,
    pub confirm: Option<String>,
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
    pub actions: Vec<FormAction>,
}

/// What the forms offer to choose from.
#[derive(Debug, Clone, Default)]
pub(crate) struct Catalog {
    pub accounts: Vec<FinanceAccount>,
    pub categories: Vec<FinanceCategory>,
    pub services: Vec<FinanceService>,
    pub reserves: Vec<FinanceSavingsReserve>,
    pub loans: Vec<LoanSummary>,
    pub merchants: Vec<(String, String)>,
    pub products: Vec<(String, String)>,
    pub today: String,
}

/// How the form treats the record it edits.
#[derive(Debug, Clone, Default)]
pub(crate) struct Editing {
    pub id: Option<String>,
    /// The document a movement belongs to: its amount, date and account
    /// are corrected there.
    pub owner: Option<&'static str>,
}

impl Editing {
    fn is_edit(&self) -> bool {
        self.id.is_some()
    }
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

/// Every kind of load, grouped as the «Cargar» menu shows them.
pub(crate) fn menu() -> Vec<MenuGroup> {
    let item = |kind, label, description| MenuItem { kind, label, description };
    vec![
        MenuGroup {
            title: "Del día a día",
            items: vec![
                item("expense", "Gasto", "Algo que pagaste, en efectivo, débito o tarjeta."),
                item("income", "Ingreso", "Plata que entró y no es el sueldo."),
                item("transfer", "Transferencia", "Entre dos cuentas tuyas de la misma moneda."),
                item("service-payment", "Pago de servicio", "Marca el servicio del mes como pagado."),
                item("loan-payment", "Pago de préstamo", "La próxima cuota de un préstamo."),
            ],
        },
        MenuGroup {
            title: "Ahorro",
            items: vec![
                item("savings-movement", "Movimiento de ahorro", "Aporte, retiro, rendimiento o ajuste de una reserva."),
                item("exchange", "Compra o venta de dólares", "Pesos que pasan al ahorro en dólares, o al revés."),
            ],
        },
        MenuGroup {
            title: "Documentos",
            items: vec![
                item("purchase", "Ticket", "Una compra con sus productos y precios."),
                item("card-statement", "Resumen de tarjeta", "Ya pagado: sus consumos cuentan en el mes del vencimiento."),
                item("salary", "Sueldo", "El recibo del mes, con o sin sus conceptos."),
                item("installment-plan", "Plan de cuotas", "Una compra en cuotas con tarjeta."),
                item("service-invoice", "Factura de servicio", "Lo que te facturó un servicio en un período."),
            ],
        },
        MenuGroup {
            title: "Configurar",
            items: vec![
                item("account", "Cuenta", "Banco, efectivo, billetera o tarjeta."),
                item("category", "Categoría", "Para ordenar gastos o ingresos."),
                item("service", "Servicio", "Algo que pagás todos los meses."),
                item("reserve", "Reserva de ahorro", "Donde guardás lo ahorrado."),
                item("loan", "Préstamo", "Lo que debés y sus cuotas."),
                item("budget", "Presupuesto", "Un tope mensual para una categoría."),
            ],
        },
    ]
}

/// The title of a kind, for messages.
pub(crate) fn kind_label(kind: &str) -> &'static str {
    menu()
        .into_iter()
        .flat_map(|group| group.items)
        .find(|item| item.kind == kind)
        .map(|item| item.label)
        .unwrap_or(match kind {
            "merchant" => "Comercio",
            "product" => "Producto",
            _ => "Registro",
        })
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

fn account_label(account: &FinanceAccount) -> String {
    let kind = match account.account_type.as_str() {
        "credit_card" => "tarjeta",
        "cash" => "efectivo",
        "wallet" => "billetera",
        "bank" => "banco",
        _ => "cuenta",
    };
    format!("{} · {kind} · {}", account.name, account.currency)
}

fn accounts(catalog: &Catalog, only_cards: bool) -> Vec<Choice> {
    catalog
        .accounts
        .iter()
        .filter(|account| account.active && (!only_cards || account.account_type == "credit_card"))
        .map(|account| choice(&account.id, account_label(account)))
        .collect()
}

fn categories(catalog: &Catalog, kind: &str, none: Option<&str>) -> Vec<Choice> {
    let mut options = none.map(|label| vec![choice("", label)]).unwrap_or_default();
    let mut found = catalog
        .categories
        .iter()
        .filter(|category| category.active && category.kind == kind)
        .map(|category| choice(&category.id, &category.name))
        .collect::<Vec<_>>();
    found.sort_by_key(|option| option.label.to_lowercase());
    options.extend(found);
    options
}

fn cents(value: &str) -> i128 {
    crate::finance::parse_cents(value).unwrap_or_default()
}

fn services(catalog: &Catalog) -> Vec<Choice> {
    catalog
        .services
        .iter()
        .filter(|service| service.active)
        .map(|service| choice(&service.id, format!("{} · {}", service.name, money_text(cents(&service.expected_amount), &service.currency))))
        .collect()
}

fn reserves(catalog: &Catalog) -> Vec<Choice> {
    catalog
        .reserves
        .iter()
        .filter(|reserve| reserve.active)
        .map(|reserve| choice(&reserve.id, format!("{} · {}", reserve.name, money_text(cents(&reserve.balance), &reserve.currency))))
        .collect()
}

fn loans(catalog: &Catalog) -> Vec<Choice> {
    catalog
        .loans
        .iter()
        .filter(|summary| summary.loan.active && summary.next_due.is_some())
        .map(|summary| {
            let next = summary.next_due.as_ref().map(|item| format!(" · cuota {}/{}", item.number, summary.loan.installment_count)).unwrap_or_default();
            choice(&summary.loan.id, format!("{}{next}", summary.loan.name))
        })
        .collect()
}

fn currencies(with_default: Option<&str>) -> Vec<Choice> {
    let mut options = with_default.map(|label| vec![choice("", label)]).unwrap_or_default();
    options.push(choice("ARS", "Pesos (ARS)"));
    options.push(choice("USD", "Dólares (USD)"));
    options
}

fn active_toggle(editing: &Editing) -> Option<Field> {
    editing.is_edit().then(|| Field::toggle("active", "Activo").default(true).help("Lo inactivo no aparece para elegir en las cargas nuevas."))
}

// ---------------------------------------------------------------------------
// Forms
// ---------------------------------------------------------------------------

fn form(kind: &str, editing: &Editing, title_new: &str, title_edit: &str, submit: &str, fields: Vec<Option<Field>>) -> EntryForm {
    EntryForm {
        kind: kind.into(),
        id: editing.id.clone(),
        title: if editing.is_edit() { title_edit } else { title_new }.into(),
        intro: None,
        submit: submit.into(),
        fields: fields.into_iter().flatten().collect(),
        delete: None,
        actions: Vec::new(),
    }
}

fn deletable(mut form: EntryForm, label: &str, confirm: &str) -> EntryForm {
    if form.id.is_some() {
        form.delete = Some(FormDelete { label: label.into(), confirm: confirm.into() });
    }
    form
}

fn intro(mut form: EntryForm, intro: Option<String>) -> EntryForm {
    form.intro = intro;
    form
}

fn movement_fields(catalog: &Catalog, editing: &Editing, category_kind: &str) -> Vec<Option<Field>> {
    let owned = editing.owner.is_some();
    let owned_help = editing.owner.map(|owner| format!("Se corrige desde el {owner}."));
    let locked = |field: Field| {
        let field = field.read_only(owned);
        match &owned_help {
            Some(help) => field.help(help.clone()),
            None => field,
        }
    };
    vec![
        Some(locked(Field::money("amount", "Importe").required().placeholder("Ej.: 15.000,50"))),
        Some(locked(Field::select("accountId", "Cuenta", accounts(catalog, false)).required())),
        Some(
            locked(Field::select("currency", "Moneda", currencies(Some("La de la cuenta"))))
                .help("Solo una tarjeta acepta pesos y dólares; las demás cuentas usan su moneda."),
        ),
        Some(locked(Field::date("effectiveDate", "Fecha").required().default(catalog.today.clone()))),
        Some(Field::select(
            "categoryId",
            "Categoría",
            categories(catalog, category_kind, Some("Sin categoría")),
        )),
        Some(Field::text("description", "Descripción").required().placeholder("Ej.: Almuerzo con amigos")),
    ]
}

fn expense(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let mut fields = movement_fields(catalog, editing, "expense");
    fields.push(Some(
        Field::select("serviceId", "Servicio", {
            let mut options = vec![choice("", "No es un servicio")];
            options.extend(services(catalog));
            options
        })
        .help("Si es el pago de un servicio, ese mes queda pagado."),
    ));
    let form = form("expense", editing, "Nuevo gasto", "Editar gasto", "Guardar gasto", fields);
    let form = intro(
        form,
        Some("Con tarjeta de crédito queda «En tarjeta, a pagar» hasta que cargues el resumen que la incluye.".into()),
    );
    deletable(form, "Eliminar gasto", "¿Eliminar este gasto? No se puede deshacer.")
}

fn income(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let form = form("income", editing, "Nuevo ingreso", "Editar ingreso", "Guardar ingreso", movement_fields(catalog, editing, "income"));
    let form = intro(form, Some("El sueldo se carga con «Sueldo» para que cuente en «¿A dónde fue el sueldo?».".into()));
    deletable(form, "Eliminar ingreso", "¿Eliminar este ingreso? No se puede deshacer.")
}

fn transfer(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::money("amount", "Importe").required()),
        Some(Field::select("accountId", "Desde", accounts(catalog, false)).required()),
        Some(Field::select("destinationAccountId", "Hacia", accounts(catalog, false)).required()),
        Some(Field::date("effectiveDate", "Fecha").required().default(catalog.today.clone())),
        Some(Field::text("description", "Descripción").required().default("Transferencia")),
    ];
    let form = form("transfer", editing, "Nueva transferencia", "Editar transferencia", "Guardar transferencia", fields);
    deletable(form, "Eliminar transferencia", "¿Eliminar esta transferencia? No se puede deshacer.")
}

fn service_payment(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("serviceId", "Servicio", services(catalog)).required()),
        Some(Field::money("amount", "Importe pagado").help("Vacío: el importe esperado del servicio.")),
        Some(Field::select("accountId", "Cuenta", {
            let mut options = vec![choice("", "La habitual del servicio")];
            options.extend(accounts(catalog, false));
            options
        })),
        Some(Field::date("effectiveDate", "Fecha de pago").required().default(catalog.today.clone())),
        Some(Field::text("description", "Descripción").placeholder("Vacío: «Pago de» y el servicio")),
    ];
    intro(
        form("service-payment", editing, "Pago de servicio", "Pago de servicio", "Registrar pago", fields),
        (catalog.services.iter().all(|service| !service.active)).then(|| "Todavía no hay servicios: creá uno en «Servicio».".to_string()),
    )
}

fn loan_payment(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("loanId", "Préstamo", loans(catalog)).required()),
        Some(Field::money("amount", "Importe pagado").help("Vacío: el importe de la cuota. Si cambió (un préstamo UVA), escribí lo que pagaste.")),
        Some(Field::select("accountId", "Cuenta", {
            let mut options = vec![choice("", "La habitual del préstamo")];
            options.extend(accounts(catalog, false));
            options
        })),
        Some(Field::date("effectiveDate", "Fecha de pago").required().default(catalog.today.clone())),
    ];
    intro(
        form("loan-payment", editing, "Pago de préstamo", "Pago de préstamo", "Pagar cuota", fields),
        Some(if catalog.loans.iter().any(|summary| summary.loan.active && summary.next_due.is_some()) {
            "Se paga la próxima cuota sin pagar del préstamo; cuenta como gasto de «Préstamos».".into()
        } else {
            "No hay préstamos con cuotas por pagar: creá uno en «Préstamo».".into()
        }),
    )
}

fn savings_movement(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let types = vec![
        choice("contribution", "Aporte"),
        choice("withdrawal", "Retiro"),
        choice("return", "Rendimiento"),
        choice("loss", "Pérdida"),
        choice("adjustment", "Ajuste"),
    ];
    let fields = vec![
        Some(Field::select("reserveId", "Reserva", reserves(catalog)).required()),
        Some(Field::select("movementType", "Tipo", types).required().default("contribution")),
        Some(Field::money("amount", "Importe").required()),
        Some(Field::select("accountId", "Cuenta", {
            let mut options = vec![choice("", "Ninguna")];
            options.extend(accounts(catalog, false));
            options
        })
        .help("De dónde salió un aporte o adónde fue un retiro.")),
        Some(Field::date("effectiveDate", "Fecha").required().default(catalog.today.clone())),
        Some(Field::text("description", "Descripción")),
        Some(Field::text("reason", "Motivo").help("Obligatorio para un retiro.")),
    ];
    let form = form("savings-movement", editing, "Movimiento de ahorro", "Editar movimiento de ahorro", "Guardar", fields);
    let form = intro(form, catalog.reserves.iter().all(|reserve| !reserve.active).then(|| "Todavía no hay reservas: creá una en «Reserva de ahorro».".to_string()));
    deletable(form, "Eliminar movimiento", "¿Eliminar este movimiento de ahorro?")
}

fn exchange(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("direction", "Operación", vec![choice("buy", "Compra: pesos al ahorro"), choice("sell", "Venta: del ahorro a una cuenta")]).required().default("buy")),
        Some(Field::select("reserveId", "Reserva", reserves(catalog)).required()),
        Some(Field::select("sourceAccountId", "Cuenta", accounts(catalog, false)).required().help("De donde salen los pesos, o adonde vuelven en una venta.")),
        Some(Field::money("sourceAmount", "Importe en la moneda de la cuenta").required().placeholder("Ej.: 1.250.000")),
        Some(Field::money("savingsAmount", "Importe en la moneda de la reserva").required().placeholder("Ej.: 1.000")),
        Some(Field::date("effectiveDate", "Fecha").required().default(catalog.today.clone())),
        Some(Field::text("description", "Descripción").default("Compra de dólares")),
    ];
    let form = form("exchange", editing, "Compra o venta de dólares", "Compra o venta de dólares", "Guardar", fields);
    intro(form, Some("No es un gasto ni un ingreso: cambia de moneda lo que tenés.".into()))
}

fn salary(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let concepts = vec![
        Field::text("name", "Concepto").required(),
        Field::select("conceptType", "Tipo", vec![choice("earning", "Haber"), choice("deduction", "Descuento")]).required().default("earning"),
        Field::money("amount", "Importe").required(),
    ];
    let fields = vec![
        Some(Field::text("employer", "Empleador").required()),
        Some(Field::month("period", "Período").required().default(catalog.today.get(..7).unwrap_or_default().to_string())),
        Some(Field::date("paymentDate", "Fecha de cobro").required().default(catalog.today.clone())),
        Some(Field::select("accountId", "Cuenta donde cobrás", accounts(catalog, false)).required()),
        Some(Field::money("netAmount", "Neto").required()),
        Some(Field::money("grossAmount", "Bruto").help("Vacío: igual al neto.")),
        Some(Field::money("deductionsTotal", "Descuentos").help("Vacío: bruto menos neto.")),
        Some(Field::items("concepts", "Conceptos del recibo", "Agregar concepto", concepts)),
    ];
    let form = form("salary", editing, "Nuevo sueldo", "Editar sueldo", "Guardar sueldo", fields);
    deletable(form, "Eliminar sueldo", "¿Eliminar este recibo de sueldo y su ingreso?")
}

fn purchase(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let items = vec![
        Field::text("originalDescription", "Producto").required(),
        Field::money("quantity", "Cantidad").default("1"),
        Field::money("unitPrice", "Precio unitario").required(),
        Field::money("lineTotal", "Total de la línea").help("Vacío: cantidad por precio."),
    ];
    let fields = vec![
        Some(Field::text("merchantName", "Comercio").required().suggestions(catalog.merchants.iter().map(|(_, name)| name.clone()).collect())),
        Some(Field::date("observedAt", "Fecha").required().default(catalog.today.clone())),
        Some(Field::select("accountId", "Cuenta", accounts(catalog, false)).required()),
        Some(Field::select("currency", "Moneda", currencies(Some("La de la cuenta")))),
        Some(Field::select("categoryId", "Categoría", categories(catalog, "expense", Some("Sin categoría")))),
        Some(Field::items("items", "Productos", "Agregar producto", items).required()),
        Some(Field::money("discountAmount", "Descuentos")),
        Some(Field::money("totalAmount", "Total pagado").help("Vacío: la suma de los productos menos los descuentos.")),
    ];
    let form = form("purchase", editing, "Nuevo ticket", "Editar ticket", "Guardar ticket", fields);
    deletable(form, "Eliminar ticket", "¿Eliminar este ticket con sus productos y su gasto?")
}

fn card_statement(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let types = vec![
        choice("purchase", "Consumo"),
        choice("fee", "Cargo"),
        choice("interest", "Interés"),
        choice("tax", "Impuesto"),
        choice("payment", "Pago"),
        choice("credit", "Crédito o devolución"),
    ];
    let lines = vec![
        Field::date("purchaseDate", "Fecha").required(),
        Field::text("description", "Descripción").required(),
        Field::money("amount", "Importe").required(),
        Field::select("itemType", "Tipo", types).required().default("purchase"),
        Field::integer("installmentNumber", "Cuota"),
        Field::integer("installmentCount", "De"),
        Field::select("categoryId", "Categoría", categories(catalog, "expense", Some("La habitual del comercio"))),
    ];
    let fields = vec![
        Some(Field::select("accountId", "Tarjeta", accounts(catalog, true)).required()),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS").help("Un resumen con pesos y dólares se carga dos veces, una por moneda.")),
        Some(Field::text("issuer", "Banco o emisor").required()),
        Some(Field::text("cardLastFour", "Últimos 4 dígitos").placeholder("Opcional")),
        Some(Field::month("period", "Período").required().default(catalog.today.get(..7).unwrap_or_default().to_string())),
        Some(Field::date("closingDate", "Cierre").required()),
        Some(Field::date("dueDate", "Vencimiento").required()),
        Some(Field::money("totalDue", "Total a pagar").required()),
        Some(Field::money("minimumPayment", "Pago mínimo")),
        Some(Field::money("previousBalance", "Saldo anterior").help("Vacío: cero.")),
        Some(Field::money("paymentsAmount", "Pagos del período").help("Vacío: cero.")),
        Some(Field::items("items", "Líneas del resumen", "Agregar línea", lines).required()),
    ];
    let form = form("card-statement", editing, "Nuevo resumen de tarjeta", "Editar resumen de tarjeta", "Guardar resumen", fields);
    let form = intro(
        form,
        Some("Cargar el resumen es decir que ya se pagó. Los consumos, cargos, intereses e impuestos se suman solos desde las líneas; los gastos con tarjeta que esperaban este resumen se unen a su línea.".into()),
    );
    deletable(form, "Eliminar resumen", "¿Eliminar este resumen? Sus gastos vuelven a quedar «En tarjeta, a pagar» o se borran.")
}

fn installment_plan(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("accountId", "Tarjeta o cuenta", accounts(catalog, false)).required()),
        Some(Field::text("merchantName", "Comercio").suggestions(catalog.merchants.iter().map(|(_, name)| name.clone()).collect())),
        Some(Field::text("description", "Descripción").required()),
        Some(Field::date("purchaseDate", "Fecha de compra").required().default(catalog.today.clone())),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS")),
        Some(Field::money("totalAmount", "Total").required()),
        Some(Field::integer("installmentCount", "Cuotas").required().default("3")),
    ];
    let form = form("installment-plan", editing, "Nuevo plan de cuotas", "Editar plan de cuotas", "Guardar plan", fields);
    let form = intro(form, Some("Las cuotas no son gastos todavía: cada una cuenta cuando la paga un resumen.".into()));
    deletable(form, "Eliminar plan", "¿Eliminar este plan de cuotas?")
}

fn service_invoice(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("serviceId", "Servicio", services(catalog)).required()),
        Some(Field::month("period", "Período").required().default(catalog.today.get(..7).unwrap_or_default().to_string())),
        Some(Field::money("amount", "Importe").required()),
        Some(Field::date("dueDate", "Vencimiento")),
        Some(Field::text("provider", "Proveedor")),
    ];
    let form = form("service-invoice", editing, "Nueva factura de servicio", "Editar factura", "Guardar factura", fields);
    let form = intro(form, Some("Registrar la factura no crea un gasto: el pago se carga con «Pago de servicio».".into()));
    deletable(form, "Eliminar factura", "¿Eliminar esta factura?")
}

fn account(_catalog: &Catalog, editing: &Editing) -> EntryForm {
    let types = vec![
        choice("bank", "Cuenta bancaria"),
        choice("cash", "Efectivo"),
        choice("wallet", "Billetera virtual"),
        choice("credit_card", "Tarjeta de crédito"),
        choice("other", "Otra"),
    ];
    let fields = vec![
        Some(Field::text("name", "Nombre").required().placeholder("Ej.: Galicia, Mercado Pago, Visa")),
        Some(Field::select("accountType", "Tipo", types).required().default("bank")),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS").help("Una tarjeta acepta pesos y dólares igual.")),
        active_toggle(editing),
    ];
    let form = form("account", editing, "Nueva cuenta", "Editar cuenta", "Guardar cuenta", fields);
    deletable(form, "Desactivar cuenta", "¿Desactivar esta cuenta? Sus movimientos quedan.")
}

fn category(_catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::text("name", "Nombre").required()),
        Some(Field::select("kind", "Para", vec![choice("expense", "Gastos"), choice("income", "Ingresos")]).required().default("expense")),
        Some(Field::multiline("description", "Qué entra").help("Le sirve al asistente para elegirla.")),
        active_toggle(editing),
    ];
    let form = form("category", editing, "Nueva categoría", "Editar categoría", "Guardar categoría", fields);
    deletable(form, "Desactivar categoría", "¿Desactivar esta categoría? Sus movimientos quedan.")
}

fn service(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::text("name", "Nombre").required().placeholder("Ej.: Internet Fibertel")),
        Some(Field::select("categoryId", "Categoría", categories(catalog, "expense", None)).required()),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS")),
        Some(Field::money("expectedAmount", "Importe esperado").required()),
        Some(Field::select("modality", "Importe", vec![choice("fixed", "Fijo"), choice("variable", "Variable")]).required().default("fixed")),
        Some(Field::integer("dueDay", "Día de vencimiento").placeholder("1 a 31")),
        Some(Field::select("defaultAccountId", "Cuenta habitual", {
            let mut options = vec![choice("", "Ninguna")];
            options.extend(accounts(catalog, false));
            options
        })),
        Some(Field::text("provider", "Proveedor")),
        active_toggle(editing),
    ];
    let form = form("service", editing, "Nuevo servicio", "Editar servicio", "Guardar servicio", fields);
    deletable(form, "Eliminar servicio", "¿Eliminar este servicio? Sus pagos quedan como gastos.")
}

fn reserve(_catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::text("name", "Nombre").required().placeholder("Ej.: Dólares, Fondo de emergencia")),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("USD")),
        Some(Field::money("openingBalance", "Saldo inicial").default("0")),
        Some(Field::text("objective", "Objetivo").placeholder("Opcional")),
        active_toggle(editing),
    ];
    let form = form("reserve", editing, "Nueva reserva", "Editar reserva", "Guardar reserva", fields);
    deletable(form, "Eliminar reserva", "¿Eliminar esta reserva con sus movimientos?")
}

fn loan(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::text("name", "Nombre").required().placeholder("Ej.: Préstamo del auto")),
        Some(Field::text("lender", "Quién te lo dio").placeholder("Banco, financiera o persona")),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS")),
        Some(Field::money("principal", "Lo que recibiste").placeholder("Opcional")),
        Some(Field::integer("installmentCount", "Cantidad de cuotas").required().default("12")),
        Some(Field::money("installmentAmount", "Importe de cada cuota").required()),
        Some(Field::date("firstDueDate", "Primer vencimiento").required().default(catalog.today.clone())),
        Some(Field::select("accountId", "Cuenta habitual de pago", {
            let mut options = vec![choice("", "Ninguna")];
            options.extend(accounts(catalog, false));
            options
        })),
        Some(Field::multiline("notes", "Notas")),
        active_toggle(editing),
    ];
    let form = form("loan", editing, "Nuevo préstamo", "Editar préstamo", "Guardar préstamo", fields);
    let mut form = intro(form, Some("Las cuotas vencen una por mes desde el primer vencimiento. Cambiar el importe o la cantidad cambia solo las cuotas sin pagar.".into()));
    if let Some(summary) = editing.id.as_ref().and_then(|id| catalog.loans.iter().find(|summary| &summary.loan.id == id)) {
        if let Some(last) = summary.installments.iter().rev().find(|item| item.transaction_id.is_some()) {
            form.actions.push(FormAction {
                id: "unpay-last".into(),
                label: format!("Desmarcar la cuota {}", last.number),
                confirm: Some(format!("¿Desmarcar la cuota {} y borrar su gasto?", last.number)),
            });
        }
    }
    deletable(form, "Eliminar préstamo", "¿Eliminar este préstamo y los gastos de sus cuotas pagadas?")
}

fn budget(catalog: &Catalog, editing: &Editing) -> EntryForm {
    let fields = vec![
        Some(Field::select("categoryId", "Categoría", categories(catalog, "expense", None)).required()),
        Some(Field::select("currency", "Moneda", currencies(None)).required().default("ARS")),
        Some(Field::money("amount", "Tope del mes").required()),
        active_toggle(editing),
    ];
    let form = form("budget", editing, "Nuevo presupuesto", "Editar presupuesto", "Guardar presupuesto", fields);
    let form = intro(form, Some("Al llegar al 80 % y al pasarte, Notia te avisa por Telegram.".into()));
    deletable(form, "Eliminar presupuesto", "¿Eliminar este presupuesto?")
}

fn rename_merge(kind: &str, others: &[(String, String)], editing: &Editing, title: &str) -> EntryForm {
    let mut targets = vec![choice("", "No unir")];
    targets.extend(
        others
            .iter()
            .filter(|(id, _)| Some(id) != editing.id.as_ref())
            .map(|(id, name)| choice(id, name)),
    );
    let fields = vec![
        Some(Field::text("name", "Nombre").required()),
        Some(Field::select("intoId", "Unir con", targets).help("Si es el mismo que otro, sus precios y compras pasan a ese.")),
    ];
    form(kind, editing, title, title, "Guardar", fields)
}

/// The form of `kind`, with `values` (a stored record, what the screen
/// preselected) over its defaults. `None` for an unknown kind.
pub(crate) fn build(kind: &str, catalog: &Catalog, editing: &Editing, values: &Values) -> Option<EntryForm> {
    let mut form = match kind {
        "expense" => expense(catalog, editing),
        "income" => income(catalog, editing),
        "transfer" => transfer(catalog, editing),
        "service-payment" => service_payment(catalog, editing),
        "loan-payment" => loan_payment(catalog, editing),
        "savings-movement" => savings_movement(catalog, editing),
        "exchange" => exchange(catalog, editing),
        "salary" => salary(catalog, editing),
        "purchase" => purchase(catalog, editing),
        "card-statement" => card_statement(catalog, editing),
        "installment-plan" => installment_plan(catalog, editing),
        "service-invoice" => service_invoice(catalog, editing),
        "account" => account(catalog, editing),
        "category" => category(catalog, editing),
        "service" => service(catalog, editing),
        "reserve" => reserve(catalog, editing),
        "loan" => loan(catalog, editing),
        "budget" => budget(catalog, editing),
        "merchant" => rename_merge("merchant", &catalog.merchants, editing, "Comercio"),
        "product" => rename_merge("product", &catalog.products, editing, "Producto"),
        _ => return None,
    };
    for field in &mut form.fields {
        if let Some(value) = values.get(&field.key).filter(|value| !value.is_null()) {
            field.value = field_value(field, value);
        }
    }
    Some(form)
}

/// `value` as the field shows it: amounts with a decimal comma, numbers as
/// text, rows with their own columns.
fn field_value(field: &Field, value: &Value) -> Value {
    match (field.kind, value) {
        ("money", Value::String(text)) => Value::String(money_input(text)),
        ("money" | "integer" | "text" | "select", Value::Number(number)) => Value::String(match field.kind {
            "money" => money_input(&number.to_string()),
            _ => number.to_string(),
        }),
        ("toggle", Value::Bool(_)) => value.clone(),
        ("date", Value::String(text)) => Value::String(text.get(..10).unwrap_or(text).to_string()),
        ("month", Value::String(text)) => Value::String(text.get(..7).unwrap_or(text).to_string()),
        ("items", Value::Array(rows)) => Value::Array(
            rows.iter()
                .filter_map(Value::as_object)
                .map(|row| {
                    let mut shown = Map::new();
                    for column in &field.columns {
                        let cell = row.get(&column.key).filter(|value| !value.is_null()).map_or_else(|| column.value.clone(), |value| field_value(column, value));
                        shown.insert(column.key.clone(), cell);
                    }
                    Value::Object(shown)
                })
                .collect(),
        ),
        (_, Value::String(_)) => value.clone(),
        _ => field.value.clone(),
    }
}

/// A stored amount (`1234.5`) as it is typed in Argentina (`1234,50`).
pub(crate) fn money_input(value: &str) -> String {
    let trimmed = value.trim();
    let Some((whole, fraction)) = trimmed.split_once('.') else {
        return trimmed.to_string();
    };
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return trimmed.to_string();
    }
    match fraction.len() {
        0 => whole.to_string(),
        1 => format!("{whole},{fraction}0"),
        _ => format!("{whole},{fraction}"),
    }
}

/// The values of a form as the builders read them: text trimmed, empty
/// text gone, the columns `integer` of item rows as numbers.
pub(crate) fn clean_values(form: &EntryForm, values: &Values) -> Values {
    let mut clean = Map::new();
    for field in &form.fields {
        let Some(value) = values.get(&field.key) else { continue };
        if let Some(value) = clean_value(field, value) {
            clean.insert(field.key.clone(), value);
        }
    }
    clean
}

fn clean_value(field: &Field, value: &Value) -> Option<Value> {
    match (field.kind, value) {
        ("toggle", Value::Bool(_)) => Some(value.clone()),
        ("toggle", Value::String(text)) => Some(Value::Bool(matches!(text.as_str(), "true" | "1" | "on"))),
        ("integer", Value::String(text)) => text.trim().parse::<u64>().ok().map(|number| json!(number)),
        ("integer", Value::Number(_)) => Some(value.clone()),
        ("items", Value::Array(rows)) => Some(Value::Array(
            rows.iter()
                .filter_map(Value::as_object)
                .map(|row| {
                    let mut cells = Map::new();
                    for column in &field.columns {
                        if let Some(cell) = row.get(&column.key).and_then(|cell| clean_value(column, cell)) {
                            cells.insert(column.key.clone(), cell);
                        }
                    }
                    Value::Object(cells)
                })
                .filter(|row| row.as_object().is_some_and(|cells| !cells.is_empty()))
                .collect(),
        )),
        (_, Value::String(text)) => Some(text.trim()).filter(|text| !text.is_empty()).map(|text| Value::String(text.to_string())),
        (_, Value::Number(_)) => Some(value.clone()),
        _ => None,
    }
}

/// The required fields left empty, with the message for each.
pub(crate) fn missing_required(form: &EntryForm, values: &Values) -> Vec<(String, String)> {
    let mut missing = Vec::new();
    for field in form.fields.iter().filter(|field| field.required && !field.read_only) {
        let empty = match values.get(&field.key) {
            None | Some(Value::Null) => true,
            Some(Value::String(text)) => text.trim().is_empty(),
            Some(Value::Array(rows)) => rows.is_empty(),
            _ => false,
        };
        if empty {
            let what = if field.kind == "items" { format!("Agregá al menos una fila en «{}».", field.label) } else { format!("Completá «{}».", field.label) };
            missing.push((field.key.clone(), what));
            continue;
        }
        if let (Some(Value::Array(rows)), false) = (values.get(&field.key), field.columns.is_empty()) {
            for (index, row) in rows.iter().enumerate() {
                for column in field.columns.iter().filter(|column| column.required) {
                    let cell_empty = row.get(&column.key).is_none_or(|cell| cell.as_str().is_some_and(|text| text.trim().is_empty()) || cell.is_null());
                    if cell_empty {
                        missing.push((field.key.clone(), format!("Completá «{}» en la fila {}.", column.label, index + 1)));
                        break;
                    }
                }
            }
        }
    }
    missing
}

/// The label of a field of `form`, for a message about it.
pub(crate) fn field_label<'a>(form: &'a EntryForm, key: &str) -> Option<&'a str> {
    form.fields.iter().find(|field| field.key == key).map(|field| field.label.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog {
            accounts: vec![
                FinanceAccount { id: "galicia".into(), name: "Galicia".into(), account_type: "bank".into(), currency: "ARS".into(), active: true },
                FinanceAccount { id: "visa".into(), name: "Visa".into(), account_type: "credit_card".into(), currency: "ARS".into(), active: true },
                FinanceAccount { id: "vieja".into(), name: "Vieja".into(), account_type: "bank".into(), currency: "ARS".into(), active: false },
            ],
            categories: vec![
                FinanceCategory { id: "super".into(), name: "Supermercado".into(), kind: "expense".into(), active: true, parent_id: None, description: None },
                FinanceCategory { id: "extra".into(), name: "Changas".into(), kind: "income".into(), active: true, parent_id: None, description: None },
            ],
            today: "2026-11-18".into(),
            ..Catalog::default()
        }
    }

    fn field<'a>(form: &'a EntryForm, key: &str) -> &'a Field {
        form.fields.iter().find(|field| field.key == key).unwrap_or_else(|| panic!("{key}"))
    }

    #[test]
    fn every_kind_of_the_menu_has_its_form() {
        for item in menu().into_iter().flat_map(|group| group.items) {
            let form = build(item.kind, &catalog(), &Editing::default(), &Values::new()).unwrap_or_else(|| panic!("{}", item.kind));
            assert_eq!(form.kind, item.kind);
            assert!(!form.fields.is_empty(), "{}", item.kind);
            assert!(form.delete.is_none(), "a new record has nothing to delete: {}", item.kind);
        }
        assert!(build("nada", &catalog(), &Editing::default(), &Values::new()).is_none());
    }

    #[test]
    fn an_expense_offers_active_accounts_and_expense_categories_and_starts_today() {
        let form = build("expense", &catalog(), &Editing::default(), &Values::new()).expect("form");
        let accounts = field(&form, "accountId").options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>();
        assert_eq!(accounts, vec!["galicia", "visa"], "an inactive account is not offered");
        let categories = field(&form, "categoryId").options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>();
        assert_eq!(categories, vec!["", "super"]);
        assert_eq!(field(&form, "effectiveDate").value, json!("2026-11-18"));
    }

    #[test]
    fn a_stored_record_fills_the_form_as_people_type_it() {
        let mut values = Values::new();
        values.insert("amount".into(), json!("15000.5"));
        values.insert("effectiveDate".into(), json!("2026-11-02T00:00:00Z"));
        values.insert("categoryId".into(), json!("super"));
        let editing = Editing { id: Some("t1".into()), owner: None };
        let form = build("expense", &catalog(), &editing, &values).expect("form");
        assert_eq!(field(&form, "amount").value, json!("15000,50"));
        assert_eq!(field(&form, "effectiveDate").value, json!("2026-11-02"));
        assert_eq!(field(&form, "categoryId").value, json!("super"));
        assert!(form.delete.is_some(), "an edited movement can be deleted");
    }

    #[test]
    fn a_movement_of_a_document_only_changes_what_the_document_does_not_own() {
        let editing = Editing { id: Some("t1".into()), owner: Some("ticket") };
        let form = build("expense", &catalog(), &editing, &Values::new()).expect("form");
        assert!(field(&form, "amount").read_only && field(&form, "accountId").read_only && field(&form, "effectiveDate").read_only);
        assert_eq!(field(&form, "amount").help.as_deref(), Some("Se corrige desde el ticket."));
        assert!(!field(&form, "categoryId").read_only && !field(&form, "description").read_only);
        let mut values = Values::new();
        values.insert("description".into(), json!("Compra"));
        assert!(
            missing_required(&form, &values).is_empty(),
            "read-only fields are not asked for: {:?}",
            missing_required(&form, &values)
        );
    }

    #[test]
    fn values_are_cleaned_and_required_ones_are_named() {
        let form = build("purchase", &catalog(), &Editing::default(), &Values::new()).expect("form");
        let mut values = Values::new();
        values.insert("merchantName".into(), json!("  Coto "));
        values.insert("items".into(), json!([{ "originalDescription": "Leche", "unitPrice": "1.200" }, { "originalDescription": "" }]));
        let clean = clean_values(&form, &values);
        assert_eq!(clean["merchantName"], json!("Coto"));
        assert_eq!(clean["items"], json!([{ "originalDescription": "Leche", "unitPrice": "1.200" }]), "an empty row goes away");
        let missing = missing_required(&form, &clean).into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        assert!(missing.contains(&"accountId".to_string()), "{missing:?}");
        assert!(!missing.contains(&"merchantName".to_string()));
        let mut incomplete = Values::new();
        incomplete.insert("items".into(), json!([{ "originalDescription": "Pan" }]));
        let messages = missing_required(&form, &incomplete);
        assert!(messages.iter().any(|(key, message)| key == "items" && message.contains("Precio unitario") && message.contains("fila 1")), "{messages:?}");
    }

    #[test]
    fn integers_of_item_rows_travel_as_numbers() {
        let form = build("card-statement", &catalog(), &Editing::default(), &Values::new()).expect("form");
        let mut values = Values::new();
        values.insert("items".into(), json!([{ "description": "Heladera", "installmentNumber": "3", "installmentCount": "12" }]));
        let clean = clean_values(&form, &values);
        assert_eq!(clean["items"][0]["installmentNumber"], json!(3));
        assert_eq!(clean["items"][0]["installmentCount"], json!(12));
        assert_eq!(field(&form, "accountId").options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>(), vec!["visa"], "only cards");
    }

    #[test]
    fn stored_amounts_read_with_a_decimal_comma() {
        assert_eq!(money_input("1234.5"), "1234,50");
        assert_eq!(money_input("1234.56"), "1234,56");
        assert_eq!(money_input("1234"), "1234");
        assert_eq!(money_input("1.234,50"), "1.234,50", "what the person typed stays");
    }
}
