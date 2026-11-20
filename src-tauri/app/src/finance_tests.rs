//! Pruebas de Finanzas de punta a punta sobre una base en memoria: lo que
//! se carga con [`finance_ops::apply`] y lo que muestran las vistas.

use serde_json::{json, Value};

use crate::finance::{add_months, current_month, today};
use crate::finance_forms as forms;
use crate::finance_model as model;
use crate::finance_ops::{self as ops, Applied, OpError, Values};
use crate::finance_screen as screen;

fn database() -> rusqlite::Connection {
    crate::finance::tests::database()
}

fn values(value: Value) -> Values {
    value.as_object().cloned().unwrap_or_default()
}

fn apply(connection: &rusqlite::Connection, kind: &str, id: Option<&str>, value: Value) -> Applied {
    ops::apply(connection, "manual", kind, id, "save", &values(value)).unwrap_or_else(|error| panic!("{kind}: {error:?}"))
}

fn rejected(connection: &rusqlite::Connection, kind: &str, value: Value) -> Vec<String> {
    match ops::apply(connection, "manual", kind, None, "save", &values(value)) {
        Err(OpError::Rejected { errors, message }) => {
            if errors.is_empty() { vec![message] } else { errors.into_keys().collect() }
        }
        other => panic!("{kind} should be rejected: {other:?}"),
    }
}

fn delete(connection: &rusqlite::Connection, kind: &str, id: &str) -> Applied {
    ops::apply(connection, "manual", kind, Some(id), "delete", &Values::new()).unwrap_or_else(|error| panic!("delete {kind}: {error:?}"))
}

fn id(applied: Applied) -> String {
    applied.saved_id.expect("saved id")
}

fn count(connection: &rusqlite::Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).expect(sql)
}

#[test]
fn an_expense_is_a_name_an_amount_and_a_category() {
    let connection = database();
    let saved = apply(&connection, "expense", None, json!({ "name": "Nafta", "amount": "45.000", "categoryId": "transporte" }));
    assert_eq!(saved.message, "Gasto guardado: Nafta · $ 45.000 · Transporte");
    let movement = model::load_movement(&connection, saved.saved_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((movement.amount.as_str(), movement.currency.as_str(), movement.date), ("45000.00", "ARS", today()));
    assert_eq!(movement.account_id, None, "accounts are optional");
    assert!(saved.undo.is_some_and(|undo| undo.action == "delete"));

    let mut missing = rejected(&connection, "expense", json!({ "amount": "100" }));
    missing.sort();
    assert_eq!(missing, ["name"]);
    assert_eq!(rejected(&connection, "expense", json!({ "name": "x", "amount": "0", "categoryId": "Otros" })), ["amount"]);
    assert_eq!(rejected(&connection, "expense", json!({ "name": "x", "amount": "10" })), ["categoryId"]);
    assert_eq!(rejected(&connection, "expense", json!({ "name": "x", "amount": "10", "categoryId": "Sueldo" })), ["categoryId"], "an income category");

    let id = movement.id;
    apply(&connection, "expense", Some(&id), json!({ "name": "Nafta YPF", "amount": "46000", "categoryId": "category-transport", "currency": "USD" }));
    let edited = model::load_movement(&connection, &id).unwrap().unwrap();
    assert_eq!((edited.name.as_str(), edited.currency.as_str()), ("Nafta YPF", "USD"));
    delete(&connection, "movement", &id);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements"), 0);
}

#[test]
fn the_salary_is_only_what_was_paid() {
    let connection = database();
    let saved = apply(&connection, "salary", None, json!({ "amount": "1.500.000" }));
    assert!(saved.message.starts_with("Sueldo guardado: $ 1.500.000"), "{}", saved.message);
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "salary", None, &Values::new()).unwrap().unwrap();
    let keys = form.fields.iter().map(|field| field.key.as_str()).collect::<Vec<_>>();
    assert_eq!(keys, ["amount", "date"], "no gross, deductions, concepts or employer");
    assert_eq!(form.fields[0].value, json!("1500000"), "it starts from the last salary");
    let edit = forms::build(&connection, &catalog, "movement", saved.saved_id.as_deref(), &Values::new()).unwrap().unwrap();
    assert_eq!(edit.kind, "salary");
    let view = screen::overview(&connection, &current_month()).unwrap();
    assert_eq!((view.balance[0].salary.as_str(), view.balance[0].left.as_str()), ("1500000.00", "1500000.00"));
}

#[test]
fn a_card_purchase_is_paid_installment_by_installment() {
    let connection = database();
    let month = current_month();
    let card = id(apply(&connection, "card", None, json!({ "name": "Visa" })));
    let saved = apply(
        &connection,
        "card-purchase",
        None,
        json!({ "cardId": "visa", "name": "Heladera", "total": "100000", "installmentCount": 3, "categoryId": "Hogar", "firstMonth": month }),
    );
    assert_eq!(saved.message, format!("Compra con Visa guardada: Heladera · $ 100.000 en 3 cuotas de $ 33.333,33 desde {}.", crate::finance::month_label(&month)));
    let purchase = id(saved);
    let states = model::purchase_states(&connection).unwrap();
    let amounts = states[0].installments.iter().map(|installment| installment.amount.as_str()).collect::<Vec<_>>();
    assert_eq!(amounts, ["33333.34", "33333.33", "33333.33"], "they add up to the total");

    // Nothing counts as spent until it is paid.
    assert_eq!(screen::overview(&connection, &month).unwrap().balance.len(), 0);
    let payments = screen::payments(&connection, &month).unwrap();
    assert_eq!(payments.cards[0].installments.len(), 1);
    assert_eq!(payments.cards[0].pending_count, 1);
    assert_eq!(payments.cards[0].debt[0].amount, "100000.00");

    // Paying the month pays what is due; again, there is nothing left.
    let paid = apply(&connection, "card-payment", None, json!({ "cardId": card }));
    assert_eq!(paid.message, "Cuota de Visa pagada: $ 33.333,34.");
    let undo = paid.undo.clone().expect("undo");
    assert_eq!(undo.kind, "movements");
    let movement = model::load_movement(&connection, paid.saved_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((movement.name.as_str(), movement.category_id.as_deref()), ("Heladera · cuota 1/3", Some("category-home")));
    assert_eq!(rejected(&connection, "card-payment", json!({ "cardId": card })).len(), 1);
    let view = screen::overview(&connection, &month).unwrap();
    assert_eq!(view.balance[0].expenses, "33333.34", "the installment counts the month it is paid");

    // Undoing the payment leaves it pending again.
    ops::apply(&connection, "manual", &undo.kind, None, &undo.action, &undo.values).expect("undo");
    assert_eq!(screen::payments(&connection, &month).unwrap().cards[0].pending_count, 1);

    // Later months: the late one and the one of the month, chosen one by one.
    let next = add_months(&month, 1);
    let due = screen::payments(&connection, &next).unwrap().cards[0].installments.clone();
    assert_eq!(due.iter().map(|installment| (installment.number, installment.late)).collect::<Vec<_>>(), [(1, true), (2, false)]);
    apply(&connection, "card-payment", None, json!({ "cardId": "Visa", "installments": [format!("{purchase}:2")] }));
    assert_eq!(rejected(&connection, "card-payment", json!({ "cardId": card, "installments": [format!("{purchase}:2")] })).len(), 1, "paid already");
    apply(&connection, "card-payment", None, json!({ "cardId": card, "month": next }));
    let state = model::purchase_state_of(&connection, &purchase).unwrap().unwrap();
    assert_eq!((state.paid_count, state.remaining.as_str()), (2, "33333.33"));

    // It cannot shrink below what was paid; deleting it keeps the payments.
    let shrink = ops::apply(&connection, "manual", "card-purchase", Some(&purchase), "save", &values(json!({ "cardId": card, "name": "Heladera", "total": "100000", "installmentCount": 1 })));
    assert!(matches!(shrink, Err(OpError::Rejected { .. })));
    delete(&connection, "card-purchase", &purchase);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements WHERE card_purchase_id IS NULL AND installment_number IS NOT NULL"), 2);
}

#[test]
fn a_purchase_creates_its_first_card_when_there_is_none() {
    let connection = database();
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "card-purchase", None, &Values::new()).unwrap().unwrap();
    assert_eq!(form.fields[0].key, "newCard");
    apply(&connection, "card-purchase", None, json!({ "newCard": "Master", "name": "Zapatillas", "total": "90000", "installmentCount": "3" }));
    assert_eq!(model::list_cards(&connection).unwrap()[0].name, "Master");
}

#[test]
fn a_loan_is_paid_with_what_each_installment_cost() {
    let connection = database();
    let month = current_month();
    let loan = id(apply(&connection, "loan", None, json!({ "name": "Préstamo banco", "amount": "600000", "installmentCount": 3, "firstMonth": month })));
    let first = apply(&connection, "loan-payment", None, json!({ "loanId": "prestamo banco", "amount": "62.000" }));
    assert_eq!(first.message, "Cuota 1/3 de Préstamo banco pagada: $ 62.000. Quedan 2.");
    apply(&connection, "loan-payment", None, json!({ "loanId": loan, "amount": "64500" }));
    let state = model::loan_state_of(&connection, &loan).unwrap().unwrap();
    assert_eq!((state.paid_count, state.total_paid.as_str(), state.last_paid.as_deref(), state.next_number), (2, "126500.00", Some("64500.00"), Some(3)));
    assert_eq!(rejected(&connection, "loan-payment", json!({ "loanId": loan, "amount": "1", "number": 1 })), ["number"]);
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "loan-payment", None, &Values::new()).unwrap().unwrap();
    let amount = form.fields.iter().find(|field| field.key == "amount").unwrap();
    assert_eq!(amount.value, json!(""), "each installment is typed");
    assert_eq!(amount.help.as_deref(), Some("La anterior fue de $ 64.500."));
    let view = screen::overview(&connection, &month).unwrap();
    assert_eq!(view.commitments.items.iter().find(|item| item.kind == "loan").and_then(|item| item.amount.clone()), Some("62000.00".into()), "this month's was paid");
    let last = apply(&connection, "loan-payment", None, json!({ "loanId": loan, "amount": "65000" }));
    assert!(last.message.ends_with("Con esta terminaste el préstamo."), "{}", last.message);
    let category: Option<String> = connection.query_row("SELECT category_id FROM finance_movements WHERE loan_id=?1 LIMIT 1", [&loan], |row| row.get(0)).unwrap();
    assert_eq!(category.as_deref(), Some("category-loans"));
}

#[test]
fn a_service_is_a_name_and_each_payment_says_what_it_cost() {
    let connection = database();
    let month = current_month();
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "service", None, &Values::new()).unwrap().unwrap();
    assert_eq!(form.fields.iter().map(|field| field.key.as_str()).collect::<Vec<_>>(), ["name"], "no amount to guess");

    // Even after its category was deleted.
    delete(&connection, "category", "category-services");
    let internet = id(apply(&connection, "service", None, json!({ "name": "Internet" })));

    // Before paying, it shows no amount: what is missing is not invented.
    let view = screen::overview(&connection, &month).unwrap();
    let pending = view.commitments.items.iter().find(|item| item.kind == "service").unwrap();
    assert_eq!((pending.status, pending.amount.as_deref()), ("pending", None));
    assert!(view.commitments.pending.is_empty());
    assert_eq!(view.commitments.pending_note.as_deref(), Some("1 servicio"));
    let payments = screen::payments(&connection, &month).unwrap();
    assert_eq!(payments.pending_note.as_deref(), Some("1 servicio"));

    // Paying asks what it cost this month.
    let catalog = forms::catalog(&connection).unwrap();
    let pay = forms::build(&connection, &catalog, "service-payment", None, &Values::new()).unwrap().unwrap();
    let amount = pay.fields.iter().find(|field| field.key == "amount").unwrap();
    assert_eq!((amount.value.clone(), amount.help.is_none()), (json!(""), true), "nothing typed for it");
    assert_eq!(rejected(&connection, "service-payment", json!({ "serviceId": "internet" })), ["amount"]);
    let paid = apply(&connection, "service-payment", None, json!({ "serviceId": "internet", "amount": "45.000" }));
    assert_eq!(paid.message, format!("Internet de {} pagado: $ 45.000.", crate::finance::month_label(&month)));
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_categories WHERE id='category-services' AND name='Servicios'"), 1);
    assert_eq!(rejected(&connection, "service-payment", json!({ "serviceId": internet, "amount": "1" })), ["month"], "one payment per month");

    // That amount is the one shown everywhere.
    let view = screen::overview(&connection, &month).unwrap();
    let done = view.commitments.items.iter().find(|item| item.kind == "service").unwrap();
    assert_eq!((done.status, done.amount.as_deref()), ("paid", Some("45000.00")));
    assert_eq!(view.commitments.pending_note, None);
    let payments = screen::payments(&connection, &month).unwrap();
    assert_eq!(payments.services[0].paid.as_ref().map(|paid| paid.amount.as_str()), Some("45000.00"));
    assert_eq!(payments.paid[0].amount, "45000.00");

    // The next month asks again; the last payment is only a reminder, with its currency.
    let next = add_months(&month, 1);
    apply(&connection, "movement", paid.saved_id.as_deref(), json!({ "amount": "46100" }));
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "service-payment", None, &values(json!({ "month": next }))).unwrap().unwrap();
    let amount = form.fields.iter().find(|field| field.key == "amount").unwrap();
    assert_eq!(amount.value, json!(""));
    assert_eq!(amount.help.as_deref(), Some("La vez anterior pagaste $ 46.100."));
    apply(&connection, "service-payment", None, json!({ "serviceId": internet, "month": next, "amount": "12", "currency": "USD" }));
    let catalog = forms::catalog(&connection).unwrap();
    let form = forms::build(&connection, &catalog, "service-payment", None, &Values::new()).unwrap().unwrap();
    assert_eq!(form.fields.iter().find(|field| field.key == "amount").and_then(|field| field.currency.as_ref()).map(|pick| pick.value.as_str()), Some("USD"));

    // A reload for another service forgets what was typed for this one.
    let reloaded = forms::build(&connection, &catalog, "service-payment", None, &values(json!({ "serviceId": internet, "amount": "999", "_changed": "serviceId" }))).unwrap().unwrap();
    assert_eq!(reloaded.fields.iter().find(|field| field.key == "amount").unwrap().value, json!(""));

    // It does not show before it existed.
    assert!(model::services_of_month(&connection, &add_months(&month, -1)).unwrap().is_empty());

    // Deleting the service keeps its payments.
    delete(&connection, "service", &internet);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements WHERE category_id='category-services'"), 2);
}

#[test]
fn categories_are_deleted_not_deactivated() {
    let connection = database();
    let fuel = id(apply(&connection, "category", None, json!({ "name": "Nafta" })));
    apply(&connection, "expense", None, json!({ "name": "YPF", "amount": "100", "categoryId": fuel }));
    apply(&connection, "budget", None, json!({ "categoryId": fuel, "amount": "1000" }));
    assert_eq!(rejected(&connection, "category", json!({ "name": "nafta" })), ["name"], "no two with the same name");
    let deleted = delete(&connection, "category", &fuel);
    assert_eq!(deleted.message, "Categoría borrada. Sus 1 movimientos quedan sin categoría.");
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements WHERE category_id IS NULL"), 1);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_budgets"), 0);
    let view = screen::overview(&connection, &current_month()).unwrap();
    assert_eq!(view.categories[0].name, "Sin categoría");
}

#[test]
fn a_budget_is_one_per_category_and_currency() {
    let connection = database();
    let first = id(apply(&connection, "budget", None, json!({ "categoryId": "Supermercado", "amount": "100000" })));
    let again = id(apply(&connection, "budget", None, json!({ "categoryId": "Supermercado", "amount": "120000" })));
    assert_eq!(first, again, "it replaces the amount");
    apply(&connection, "expense", None, json!({ "name": "Coto", "amount": "100000", "categoryId": "Supermercado" }));
    let rows = crate::finance_budgets::budgets_of_month(&connection, &current_month()).unwrap();
    assert_eq!((rows[0].percent, rows[0].status), (83.3, "near"));
    let view = screen::overview(&connection, &current_month()).unwrap();
    assert_eq!(view.categories[0].budget.as_ref().map(|budget| budget.status), Some("near"));
}

#[test]
fn savings_and_dollars_count_in_the_month() {
    let connection = database();
    let month = current_month();
    apply(&connection, "income", None, json!({ "name": "Venta", "amount": "500000" }));
    apply(&connection, "savings", None, json!({ "reserveId": "Ahorro en pesos", "type": "deposit", "amount": "100000" }));
    let bought = apply(&connection, "exchange", None, json!({ "type": "buy", "dollars": "100", "pesos": "120.000" }));
    assert_eq!(bought.message, "Compraste US$ 100 por $ 120.000 ($ 1.200 cada dólar).");
    assert_eq!(rejected(&connection, "savings", json!({ "reserveId": "reserve-pesos", "type": "withdrawal", "amount": "100001" })), ["amount"]);
    let view = screen::overview(&connection, &month).unwrap();
    let pesos = &view.balance[0];
    assert_eq!((pesos.saved.as_str(), pesos.left.as_str()), ("220000.00", "280000.00"));
    assert_eq!(view.savings.iter().map(|total| (total.currency.as_str(), total.balance.as_str())).collect::<Vec<_>>(), [("ARS", "100000.00"), ("USD", "100.00")]);
    let savings = screen::savings(&connection).unwrap();
    assert_eq!(savings.movements.iter().filter(|row| row.kind == "exchange").count(), 1);
    let catalog = forms::catalog(&connection).unwrap();
    let edit = forms::build(&connection, &catalog, "savings", Some(&savings.movements[0].id), &Values::new()).unwrap().unwrap();
    assert_eq!(edit.kind, "exchange", "an exchange opens its own form");
}

#[test]
fn the_overview_says_how_the_month_goes() {
    let connection = database();
    crate::finance::seed_demo_data(&connection).unwrap();
    let month = current_month();
    let view = screen::overview(&connection, &month).unwrap();
    let pesos = &view.balance[0];
    assert_eq!(pesos.currency, "ARS");
    assert_eq!(pesos.salary, "1500000.00");
    assert!(pesos.expenses_change.is_some(), "compared with the month before");
    assert!(view.pace.as_ref().is_some_and(|pace| pace.projected.is_some()));
    assert!(view.commitments.items.iter().any(|item| item.kind == "service" && item.status == "paid"));
    assert!(view.commitments.items.iter().any(|item| item.kind == "card" && item.status == "pending"), "the fridge's second installment is due");
    assert!(view.commitments.items.iter().any(|item| item.kind == "loan"));
    assert_eq!(view.history.len(), 6);
    assert_eq!(view.debts.upcoming.len(), 3);
    assert!(!view.debts.cards.is_empty() && !view.debts.loans.is_empty());
    assert!(!view.empty);
    let movements = screen::movements_view(&connection, &month, &screen::MovementFilter { search: "demo".into(), ..Default::default() }).unwrap();
    assert!(!movements.rows.is_empty());
    let history = screen::search(&connection, &screen::MovementFilter { search: "Supermercado".into(), ..Default::default() }, 0).unwrap();
    assert_eq!(history.count, 2, "both months");
    let sections = crate::finance_entry::manage_sections(&connection).unwrap();
    assert_eq!(sections.iter().map(|section| section.key).collect::<Vec<_>>(), [
        "cards",
        "purchases",
        "services",
        "loans",
        "budgets",
        "expense-categories",
        "income-categories",
        "reserves",
        "accounts"
    ]);
}

#[test]
fn every_form_builds_and_its_kind_saves() {
    let connection = database();
    crate::finance::seed_demo_data(&connection).unwrap();
    let catalog = forms::catalog(&connection).unwrap();
    for group in forms::menu() {
        for item in group.items {
            let form = forms::build(&connection, &catalog, item.kind, None, &Values::new()).unwrap().unwrap_or_else(|| panic!("{}", item.kind));
            assert_eq!(form.kind, item.kind);
            assert!(form.fields.iter().all(|field| !field.key.is_empty()), "{}", item.kind);
            assert!(form.delete.is_none(), "a new {} cannot be deleted", item.kind);
        }
    }
    let form = forms::build(&connection, &catalog, "card-payment", None, &Values::new()).unwrap().unwrap();
    let checks = form.fields.iter().find(|field| field.key == "installments").unwrap();
    assert_eq!(checks.kind, "checks");
    assert_eq!(checks.value.as_array().map(Vec::len), Some(checks.options.len()), "all chosen at first");
    let payment = model::list_purchases(&connection).unwrap();
    assert!(!payment.is_empty());
    // A payment opens its own form, which undoes it.
    let form = forms::build(&connection, &catalog, "movement", Some("dev-move-fridge-1"), &Values::new()).unwrap().unwrap();
    assert_eq!(form.kind, "movement");
    assert_eq!(form.delete.as_ref().map(|delete| delete.label.as_str()), Some("Deshacer el pago"));
    assert_eq!(form.fields.iter().find(|field| field.key == "amount").unwrap().value, json!("60000"));
}

/// A library on disk with the app around it, as the commands see it.
fn library() -> (crate::host::AppHandle, crate::finance::FinanceContext, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("notia-finance-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join(".notia")).expect("library");
    let app = crate::create_app(crate::host::AppPaths::new(Some(root.join("app")), None), crate::host::HostPorts::default());
    let context = crate::finance::FinanceContext {
        library_path: root.display().to_string(),
        android_directory_uri: None,
        actor_library_user_id: "user-owner".into(),
        source: "android".into(),
    };
    (app, context, root)
}

#[test]
fn the_queue_of_the_app_cannot_duplicate_a_load() {
    use crate::finance_entry::{finance_apply, finance_form, ApplyPayload, FormPayload};
    let (app, context, root) = library();
    let apply = |value: Value, request: Option<&str>| {
        finance_apply(
            app.clone(),
            ApplyPayload {
                context: context.clone(),
                kind: "expense".into(),
                id: None,
                action: None,
                values: values(value),
                request_id: request.map(str::to_string),
            },
        )
        .expect("apply")
    };
    let first = apply(json!({ "name": "Nafta", "amount": "45000", "categoryId": "Transporte" }), Some("pedido-1"));
    assert!(first.ok && !first.duplicate, "{first:?}");
    let again = apply(json!({ "name": "Nafta", "amount": "45000", "categoryId": "Transporte" }), Some("pedido-1"));
    assert!(again.ok && again.duplicate);
    assert_eq!(again.saved_id, first.saved_id);

    // A rejection writes nothing and its id can be sent again once fixed.
    let wrong = apply(json!({ "name": "Kiosco", "amount": "abc", "categoryId": "Otros" }), Some("pedido-2"));
    assert!(!wrong.ok && wrong.errors.contains_key("amount"), "{wrong:?}");
    let fixed = apply(json!({ "name": "Kiosco", "amount": "1500", "categoryId": "Otros" }), Some("pedido-2"));
    assert!(fixed.ok && !fixed.duplicate);
    let connection = crate::finance::validate_context(&context, &app).expect("connection");
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements"), 2);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM finance_movements WHERE source='android'"), 2, "the app keeps its origin");

    // A form edits what was saved.
    let form = finance_form(app.clone(), FormPayload { context: context.clone(), kind: "movement".into(), id: first.saved_id.clone(), prefill: Values::new() }).expect("form");
    assert_eq!((form.kind.as_str(), form.fields[0].value.clone()), ("expense", json!("Nafta")));
    let gone = finance_form(app.clone(), FormPayload { context: context.clone(), kind: "movement".into(), id: Some("nope".into()), prefill: Values::new() });
    assert!(gone.is_err_and(|error| error.code == "notFound"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
