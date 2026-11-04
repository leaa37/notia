use std::collections::HashSet;
use std::sync::OnceLock;

use super::context::{BackendChannel, BackendRequestContext, BackendScope};
use super::error::{BackendError, BackendErrorCode};
use super::protocol::{ToolAccess, ToolDefinition};

const CONFIDENTIAL_CONTEXT: &str = "#Confidencial";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    Public,
    LibraryRead,
    LibraryWrite,
    TaskRead,
    TaskWrite,
    FinanceRead,
    FinanceWrite,
    RoutineRead,
    RoutineWrite,
    /// The Agenda of Notia (events on the day grid and the notepad), per
    /// library user like Rutina.
    AgendaRead,
    AgendaWrite,
    /// Recetas: the recipe files of the library.
    RecipeRead,
    RecipeWrite,
    /// Salud: weight, meals, water and body measurements, per library user.
    HealthRead,
    HealthWrite,
    /// Gimnasio: the user's routines, sessions and workouts, and the
    /// exercise and equipment files of the library.
    GymRead,
    GymWrite,
    /// The AI actions of the library: the Owner's only.
    AiActionRead,
    AiActionWrite,
    /// The AI tasks of the library: the Owner's only, created by the agent.
    AiTaskRead,
    AiTaskWrite,
    /// The open Meeting of this device: the Owner's only.
    MeetingRead,
    MeetingWrite,
    Memory,
    /// Gmail and Google Calendar of the library's connected accounts;
    /// confidential, like finance.
    Mail,
    /// Images generated or edited with the local model: the Owner's, in
    /// the app chats and Telegram.
    Images,
    /// Commands in the terminal of the host PC: the Owner's, in the app
    /// chats, Telegram and the AI tasks; never a published board.
    Terminal,
    /// The web browser of the host PC (Playwright): the Owner's, where the
    /// terminal is.
    Browser,
    /// The Owner's WhatsApp linked on the host PC: the Owner's, where the
    /// terminal is, and only in the chats the Owner allowed.
    WhatsApp,
    /// The Python support scripts of `.agent/scripts`, run on the host PC:
    /// the Owner's, where the terminal is.
    Scripts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCatalogProjection {
    Full,
    ReadOnly,
    PublishedTaskManager,
}

/// Returns the catalog owned by the backend.
///
/// Requests received from a client may contain a projection (or, while the
/// migration is in progress, an old client-side catalog), but they must not be
/// able to change a tool's policy, mutability or confirmation requirements.
/// Keeping the source of truth here also gives headless transports the same
/// catalog as the Tauri transport.
pub fn canonical_tool_catalog() -> Vec<ToolDefinition> {
    let mut catalog = Vec::new();
    catalog.extend(super::library_tools::library_tool_contracts());
    catalog.extend(super::library_tools::library_mutation_tool_contracts());
    catalog.extend(super::task_manager_tools::task_manager_read_tool_contracts());
    catalog.extend(super::task_manager_tools::task_manager_mutation_tool_contracts());
    catalog.extend([
        public_tool(
            "search_web",
            "Busca fuentes públicas y devuelve enlaces verificables.",
            vec![BackendScope::Library, BackendScope::Finance],
        ),
        public_tool(
            "get_weather",
            "Consulta el clima actual y el pronóstico de Open-Meteo.",
            vec![BackendScope::Library, BackendScope::Finance],
        ),
        public_tool(
            "request_user_clarification",
            "Solicita un dato que falta antes de continuar.",
            vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager, BackendScope::Finance],
        ),
        public_tool(
            "request_user_confirmation",
            "Prepara una decisión explícita del usuario.",
            vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager, BackendScope::Finance],
        ),
        public_tool(
            "get_workspace_context",
            "Obtiene el contexto visual autorizado de la solicitud.",
            vec![BackendScope::Library, BackendScope::Document, BackendScope::Graph],
        ),
        memory_tool("add_agent_rule", "Guarda una regla operativa persistente."),
        memory_tool("add_agent_memory", "Guarda un hecho persistente del usuario."),
        memory_tool(
            "add_agent_thought",
            "Guarda un pensamiento propio: algo que observaste, avisaste, preguntaste o propusiste.",
        ),
        memory_tool("add_agent_biography", "Suma un hecho de la vida del usuario a su biografía narrada."),
        memory_tool("add_agent_talk", "Anota un rasgo de cómo habla el usuario."),
        memory_tool("add_agent_person", "Anota quién es una persona que menciona el usuario."),
        memory_tool(
            "add_agent_improvement",
            "Anota en improve.md un error de una herramienta, una herramienta que falta o algo de Notia a mejorar.",
        ),
    ]);
    catalog.extend(alias_tools(
        [
            "read_active_markdown_document",
            "request_file_read_permission",
            "extract_document_facts",
        ],
        BackendScope::Document,
        true,
    ));
    catalog.extend(finance_tools(
        [
            "get_finance_dashboard",
            "get_finance_dollar_quotes",
            "get_finance_inflation_indices",
            "get_finance_historical_dollar_quotes",
            "list_finance_accounts",
            "list_finance_categories",
            "list_finance_movements",
            "search_finance_categories",
            "list_finance_credit_card_statements",
            "list_finance_salaries",
            "list_finance_purchases",
            "list_finance_price_history",
            "list_finance_services",
            "list_finance_service_occurrences",
            "list_finance_service_invoices",
            "list_finance_review_items",
            "list_finance_products",
            "list_finance_merchants",
            "get_finance_full_snapshot",
            "get_finance_record",
            "list_finance_records",
        ],
        true,
    ));
    catalog.extend(finance_tools(
        [
            "create_finance_transaction",
            "create_finance_savings_movement",
            "create_finance_savings_exchange",
            "create_finance_category",
            "create_finance_purchase",
            "create_finance_salary",
            "create_finance_credit_card_statement",
            "update_finance_transaction_status",
            "create_finance_service",
            "create_finance_service_occurrence",
            "create_finance_service_invoice",
            "save_finance_account",
            "save_finance_category",
            "save_finance_transaction",
            "save_finance_savings_reserve",
            "save_finance_savings_movement",
            "save_finance_savings_exchange",
            "save_finance_purchase",
            "save_finance_salary",
            "save_finance_credit_card_statement",
            "save_finance_installment_plan",
            "save_finance_service",
            "save_finance_service_occurrence",
            "save_finance_service_invoice",
            "link_finance_savings_account",
            "set_finance_service_active",
            "delete_finance_record",
            "reverse_finance_transaction",
            "clear_finance_data",
            "extract_finance_document",
            "link_finance_records",
            "unlink_finance_records",
            "resolve_finance_review_item",
            "rename_finance_product",
            "rename_finance_merchant",
            "merge_finance_products",
            "merge_finance_merchants",
        ],
        false,
    ));
    catalog.extend(finance_tools(
        [
            "list_finance_installment_plans",
            "list_finance_installments",
            "list_finance_artifacts",
        ],
        true,
    ));
    catalog.extend(routine_tools(
        &[
            "get_routine_dashboard",
            "get_routine_day",
            "list_routine_history",
            "get_routine_month_report",
        ],
        true,
    ));
    catalog.extend(routine_tools(
        &[
            "save_routine",
            "delete_routine",
            "save_routine_task",
            "set_routine_task_status",
            "delete_routine_task",
            "restore_routine_task",
            "reorder_routine_tasks",
            "set_routine_completions",
            "set_routine_goal",
        ],
        false,
    ));
    catalog.extend(routine_tools(&["list_agenda"], true));
    catalog.extend(routine_tools(
        &[
            "create_agenda_event",
            "delete_agenda_event",
            "add_agenda_note",
            "set_agenda_note_done",
            "delete_agenda_note",
        ],
        false,
    ));
    catalog.extend(recipe_tools(&super::recipes::tools::RECIPE_READ_TOOLS, true));
    catalog.extend(recipe_tools(&super::recipes::tools::RECIPE_WRITE_TOOLS, false));
    // Salud is per library user, as Rutina: the same chats reach it and each
    // turn gets only the areas it needs (see `tool_routing`).
    catalog.extend(routine_tools(&super::health::tools::HEALTH_READ_TOOLS, true));
    catalog.extend(routine_tools(&super::health::tools::HEALTH_WRITE_TOOLS, false));
    // Gimnasio is per library user too; its catalog is library files.
    catalog.extend(routine_tools(&super::gym::tools::GYM_READ_TOOLS, true));
    catalog.extend(routine_tools(&super::gym::tools::GYM_WRITE_TOOLS, false));
    catalog.extend(ai_action_tools(&super::ai_actions::tools::AI_ACTION_READ_TOOLS, true));
    catalog.extend(ai_action_tools(&super::ai_actions::tools::AI_ACTION_WRITE_TOOLS, false));
    // The AI tasks reach the same chats as the actions.
    catalog.extend(ai_action_tools(&super::ai_tasks::tools::AI_TASK_READ_TOOLS, true));
    catalog.extend(ai_action_tools(&super::ai_tasks::tools::AI_TASK_WRITE_TOOLS, false));
    // Only the executor of a task keeps it (`phase_tools`); it needs no
    // confirmation: it only leaves the task waiting.
    catalog.extend(ai_action_tools(&[super::ai_tasks::waits::WAIT_TOOL], true));
    catalog.extend(meeting_tools(&super::meeting_tools::MEETING_READ_TOOLS, true));
    catalog.extend(meeting_tools(&super::meeting_tools::MEETING_WRITE_TOOLS, false));
    catalog.extend(alias_tools(
        ["undo_ai_operation"],
        BackendScope::Library,
        false,
    ));
    catalog.extend(document_mutation_alias_tools([
        "create_library_note",
        "replace_library_document",
        "delete_library_document",
        "move_library_entry",
    ]));
    catalog.extend(plan_tools());
    catalog.extend(super::mail_tools::mail_tool_contracts());
    catalog.extend(image_tools());
    catalog.push(terminal_tool());
    catalog.extend(browser_tools());
    catalog.extend(whatsapp_tools());
    catalog.extend(script_tools());
    catalog.into_iter().map(with_canonical_schema).collect()
}

/// Model-facing description and JSON schema per tool, owned by the backend
/// (`defaults/tool_schemas.json`). Policy, scopes, mutability and
/// confirmation stay in this module; the resource only documents arguments.
fn tool_schemas() -> &'static serde_json::Map<String, serde_json::Value> {
    static SCHEMAS: OnceLock<serde_json::Map<String, serde_json::Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        serde_json::from_str(include_str!("defaults/tool_schemas.json"))
            .expect("defaults/tool_schemas.json must be a JSON object")
    })
}

fn with_canonical_schema(mut tool: ToolDefinition) -> ToolDefinition {
    if let Some(entry) = tool_schemas().get(&tool.name) {
        if let Some(description) = entry.get("description").and_then(serde_json::Value::as_str) {
            tool.description = description.to_string();
        }
        if let Some(parameters) = entry.get("parameters") {
            tool.input_schema = parameters.clone();
        }
    }
    tool
}

/// Image tools write nothing to the library: the image goes with the reply
/// (the chat file, or a Telegram photo), so they ask for no confirmation.
/// They are not read-only either, so read-only chats and Notia's own runs
/// do not get them.
fn image_tools() -> Vec<ToolDefinition> {
    super::image_generation::IMAGE_TOOLS
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Genera o edita una imagen con el modelo local.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
            read_only: false,
            requires_confirmation: false,
        })
        .collect()
}

/// The terminal may change anything on the PC: every command asks for
/// confirmation («Confirmar todos» lets the rest of the request run).
fn terminal_tool() -> ToolDefinition {
    ToolDefinition {
        name: super::terminal::TERMINAL_TOOL.to_string(),
        description: "Ejecuta un comando en la terminal de la PC host.".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
        read_only: false,
        requires_confirmation: true,
    }
}

/// The browser writes nothing to the library, but a click or a key may buy,
/// send or delete on a site: those ask for confirmation («Confirmar todos»
/// lets the rest of the request run); opening and reading pages does not.
/// None is read-only, so read-only chats and Notia's own runs (the hourly
/// review, a mail that arrived) never browse.
fn browser_tools() -> Vec<ToolDefinition> {
    super::browser::BROWSER_TOOLS
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Maneja el navegador web de la PC host.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
            read_only: false,
            requires_confirmation: super::browser::is_browser_interaction_tool(name),
        })
        .collect()
}

/// Reading WhatsApp changes nothing; a message or a file reaches other
/// people, so sending asks for confirmation («Confirmar todos» lets the
/// rest of the request run). Which chats each tool may use is the Owner's
/// choice in Settings, checked when it runs.
fn whatsapp_tools() -> Vec<ToolDefinition> {
    super::whatsapp::WHATSAPP_TOOLS
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Usa el WhatsApp vinculado del Owner.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
            read_only: !super::whatsapp::is_whatsapp_send_tool(name),
            requires_confirmation: super::whatsapp::is_whatsapp_send_tool(name),
        })
        .collect()
}

/// The support scripts run code on the host PC: saving one (new code)
/// asks for confirmation with the whole file, and running a saved one does
/// not, as with any tool. Listing them is the only read-only one, so the
/// hourly review and other read-only runs never run a script.
fn script_tools() -> Vec<ToolDefinition> {
    super::agent_scripts::SCRIPT_TOOLS
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Usa los scripts Python de soporte del agente.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
            read_only: name == super::agent_scripts::LIST_TOOL,
            requires_confirmation: name == super::agent_scripts::SAVE_TOOL,
        })
        .collect()
}

/// Plan tools open an approval interaction; they never mutate by themselves.
fn plan_tools() -> Vec<ToolDefinition> {
    [
        ("set_agent_execution_plan", vec![BackendScope::Library, BackendScope::Document]),
        ("create_agent_plan", vec![BackendScope::Library, BackendScope::Document]),
        ("update_agent_plan", vec![BackendScope::Library, BackendScope::Document]),
        ("set_task_execution_plan", vec![BackendScope::TaskManager]),
    ]
    .into_iter()
    .map(|(name, scopes)| ToolDefinition {
        name: name.to_string(),
        description: "Propone un plan de pasos para aprobación del usuario.".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        scopes,
        read_only: false,
        requires_confirmation: false,
    })
    .collect()
}

/// Projects the canonical backend catalog for a request. `requested_names` is
/// only a filter; it is never a source of tool metadata or authorization.
pub fn project_canonical_tool_catalog(
    context: &BackendRequestContext,
    principal: &AuthorizationPrincipal,
    requested_names: &[String],
    projection: ToolCatalogProjection,
) -> Result<Vec<ToolDefinition>, BackendError> {
    let requested = requested_names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>();
    let canonical = canonical_tool_catalog();
    if requested_names.len() != requested.len() {
        return Err(BackendError::invalid_input(
            "La selección de tools contiene nombres vacíos o repetidos.",
        ));
    }
    if let Some(unknown) = requested
        .iter()
        .find(|name| !canonical.iter().any(|tool| tool.name == **name))
    {
        return Err(BackendError::invalid_input(format!(
            "La tool solicitada no pertenece al catálogo backend: {unknown}."
        )));
    }
    project_tool_catalog(
        context,
        principal,
        &canonical
            .into_iter()
            .filter(|tool| requested.is_empty() || requested.contains(tool.name.as_str()))
            .collect::<Vec<_>>(),
        projection,
    )
}

fn public_tool(name: &str, description: &str, scopes: Vec<BackendScope>) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        scopes,
        read_only: true,
        requires_confirmation: false,
    }
}

fn memory_tool(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        // Every app chat of the owner can save memories and rules; finance
        // stays out by design and graph chats are read-only.
        scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::TaskManager],
        read_only: false,
        requires_confirmation: false,
    }
}

fn alias_tools<const N: usize>(
    names: [&str; N],
    scope: BackendScope,
    read_only: bool,
) -> Vec<ToolDefinition> {
    names
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![scope.clone()],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

/// Finance tools serve the Finanzas chat and, routed by area
/// (`tool_routing`), the library chat and Telegram, so one request can mix
/// finance with mail or notes.
fn finance_tools<const N: usize>(names: [&str; N], read_only: bool) -> Vec<ToolDefinition> {
    names
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Finance, BackendScope::Library],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

/// Rutina and the Agenda are reachable from the library chat and from
/// Finanzas, because Telegram routes a message to Finanzas by its wording
/// ("pagué la cuenta").
fn routine_tools(names: &[&str], read_only: bool) -> Vec<ToolDefinition> {
    names
        .iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Finance],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

/// The Meeting tools work on the open meeting of this device, from the
/// library chats (the side chat over Meeting, the main chat, Telegram).
fn meeting_tools(names: &[&str], read_only: bool) -> Vec<ToolDefinition> {
    names
        .iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

/// Recetas live in the library: the library chat, Telegram and the chat of
/// Finanzas reach them; a photo of a dish sent by Telegram becomes a recipe.
fn recipe_tools(names: &[&str], read_only: bool) -> Vec<ToolDefinition> {
    names
        .iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Finance],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

/// The AI actions are the agent's own schedule, so every chat of the app
/// and Telegram reach them; the policy keeps them to the Owner.
fn ai_action_tools(names: &[&str], read_only: bool) -> Vec<ToolDefinition> {
    names
        .iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Tool versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![
                BackendScope::Library,
                BackendScope::Finance,
                BackendScope::TaskManager,
                BackendScope::Document,
                BackendScope::Graph,
            ],
            read_only,
            requires_confirmation: !read_only,
        })
        .collect()
}

fn document_mutation_alias_tools<const N: usize>(names: [&str; N]) -> Vec<ToolDefinition> {
    names
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.to_string(),
            description: "Mutación documental versionada del catálogo backend.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![BackendScope::Library, BackendScope::Document, BackendScope::Graph],
            read_only: false,
            requires_confirmation: true,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationPrincipal {
    pub library_id: String,
    pub library_user_id: String,
    pub allowed_contexts: Vec<String>,
    pub all_contexts: bool,
}

impl AuthorizationPrincipal {
    pub fn validate(&self) -> Result<(), BackendError> {
        if self.library_id.trim().is_empty()
            || self.library_user_id.trim().is_empty()
            || self
                .allowed_contexts
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(BackendError::invalid_input(
                "El principal de autorización no es válido.",
            ));
        }
        Ok(())
    }

    fn can_access_confidential_context(&self) -> bool {
        self.all_contexts
            || self
                .allowed_contexts
                .iter()
                .any(|value| value.eq_ignore_ascii_case(CONFIDENTIAL_CONTEXT))
    }
}

/// Keeps the tools a chat allows: all, only those that read, or none, and,
/// without library search, none of those that read or search the library.
/// Memory tools stay: whether they are offered depends on the memory policy.
pub fn restrict_tool_access(tools: Vec<ToolDefinition>, access: ToolAccess, library_search: bool) -> Vec<ToolDefinition> {
    tools
        .into_iter()
        .filter(|tool| {
            let policy = tool_policy(&tool.name);
            policy == ToolPolicy::Memory
                || (library_search || policy != ToolPolicy::LibraryRead)
                    && match access {
                        ToolAccess::All => true,
                        ToolAccess::ReadOnly => tool.read_only,
                        ToolAccess::None => false,
                    }
        })
        .collect()
}

pub fn tool_policy(tool_name: &str) -> ToolPolicy {
    match tool_name {
        name if super::mail_tools::is_mail_tool(name) => ToolPolicy::Mail,
        "search_web"
        | "get_weather"
        | "request_user_clarification"
        | "request_user_confirmation"
        | "get_workspace_context" => ToolPolicy::Public,
        "add_agent_rule"
        | "add_agent_memory"
        | "add_agent_thought"
        | "add_agent_biography"
        | "add_agent_talk"
        | "add_agent_person"
        | "add_agent_improvement" => {
            ToolPolicy::Memory
        }
        "search_library_documents"
        | "search_library_context"
        | "search_library_exact"
        | "get_document_metadata"
        | "inspect_page_chain"
        | "find_document_references"
        | "compare_documents"
        | "extract_document_facts"
        | "read_library_documents"
        | "read_active_markdown_document"
        | "request_file_read_permission" => ToolPolicy::LibraryRead,
        "read_all_task_tickets"
        | "search_task_tickets"
        | "search_task_context"
        | "read_task_tickets"
        | "get_task_manager_options"
        | "get_task_board_summary" => ToolPolicy::TaskRead,
        name if super::task_board_tool_input::is_task_board_read_tool(name) => ToolPolicy::TaskRead,
        name if super::task_board_tool_input::is_task_board_action_tool(name) => ToolPolicy::TaskWrite,
        "set_agent_execution_plan"
        | "set_task_execution_plan"
        | "create_agent_plan"
        | "update_agent_plan" => ToolPolicy::TaskWrite,
        name if super::task_manager_tool_input::is_task_mutation_tool(name) => ToolPolicy::TaskWrite,
        "get_routine_dashboard"
        | "get_routine_day"
        | "list_routine_history"
        | "get_routine_month_report" => ToolPolicy::RoutineRead,
        "save_routine"
        | "delete_routine"
        | "save_routine_task"
        | "set_routine_task_status"
        | "delete_routine_task"
        | "restore_routine_task"
        | "reorder_routine_tasks"
        | "set_routine_completions"
        | "set_routine_goal" => ToolPolicy::RoutineWrite,
        name if super::recipes::tools::RECIPE_READ_TOOLS.contains(&name) => ToolPolicy::RecipeRead,
        name if super::recipes::tools::is_recipe_write_tool(name) => ToolPolicy::RecipeWrite,
        name if super::health::tools::HEALTH_READ_TOOLS.contains(&name) => ToolPolicy::HealthRead,
        name if super::health::tools::is_health_write_tool(name) => ToolPolicy::HealthWrite,
        name if super::gym::tools::GYM_READ_TOOLS.contains(&name) => ToolPolicy::GymRead,
        name if super::gym::tools::is_gym_write_tool(name) => ToolPolicy::GymWrite,
        name if super::ai_actions::tools::AI_ACTION_READ_TOOLS.contains(&name) => ToolPolicy::AiActionRead,
        name if super::ai_tasks::tools::AI_TASK_READ_TOOLS.contains(&name) || name == super::ai_tasks::waits::WAIT_TOOL => ToolPolicy::AiTaskRead,
        name if super::ai_tasks::tools::is_ai_task_write_tool(name) => ToolPolicy::AiTaskWrite,
        name if super::ai_actions::tools::is_ai_action_write_tool(name) => ToolPolicy::AiActionWrite,
        name if super::meeting_tools::MEETING_READ_TOOLS.contains(&name) => ToolPolicy::MeetingRead,
        name if super::meeting_tools::is_meeting_write_tool(name) => ToolPolicy::MeetingWrite,
        name if super::image_generation::is_image_tool(name) => ToolPolicy::Images,
        name if super::terminal::is_terminal_tool(name) => ToolPolicy::Terminal,
        name if super::browser::is_browser_tool(name) => ToolPolicy::Browser,
        name if super::whatsapp::is_whatsapp_tool(name) => ToolPolicy::WhatsApp,
        name if super::agent_scripts::is_script_tool(name) => ToolPolicy::Scripts,
        "list_agenda" => ToolPolicy::AgendaRead,
        "create_agenda_event"
        | "delete_agenda_event"
        | "add_agenda_note"
        | "set_agenda_note_done"
        | "delete_agenda_note" => ToolPolicy::AgendaWrite,
        name if name.starts_with("list_finance_") || name.starts_with("get_finance_") => {
            ToolPolicy::FinanceRead
        }
        name if name.starts_with("create_finance_")
            || name.starts_with("save_finance_")
            || name.starts_with("update_finance_")
            || name.starts_with("set_finance_")
            || name.starts_with("delete_finance_")
            || name.starts_with("reverse_finance_")
            || name.starts_with("clear_finance_")
            || name.starts_with("link_finance_")
            || name.starts_with("unlink_finance_")
            || name.starts_with("resolve_finance_")
            || name.starts_with("rename_finance_")
            || name.starts_with("merge_finance_")
            || name == "extract_finance_document" =>
        {
            ToolPolicy::FinanceWrite
        }
        _ => ToolPolicy::LibraryWrite,
    }
}

fn is_task_scope(tool: &ToolDefinition) -> bool {
    tool.scopes.is_empty() || tool.scopes.contains(&BackendScope::TaskManager)
}

/// What a published Task Manager's guest may use: the Task Manager tools
/// and the questions to the person. Tools without a scope (mail, agenda,
/// the agent's memory, images) stay out.
fn is_published_tool(tool: &ToolDefinition) -> bool {
    matches!(tool.name.as_str(), "request_user_clarification" | "request_user_confirmation")
        || (is_task_scope(tool) && crate::tool_routing::tool_area(&tool.name) == Some(crate::tool_routing::ToolArea::Tasks))
}

pub fn authorize_tool_call(
    context: &BackendRequestContext,
    principal: &AuthorizationPrincipal,
    tool: &ToolDefinition,
    projection: ToolCatalogProjection,
) -> Result<(), BackendError> {
    context.validate()?;
    principal.validate()?;
    if principal.library_id != context.library_id
        || principal.library_user_id != context.actor.library_user_id
    {
        return Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta no está autorizada para esta biblioteca o usuario.",
            false,
        ));
    }
    if !tool.scopes.is_empty() && !tool.scopes.contains(&context.scope) {
        return Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta no está autorizada para este scope.",
            false,
        ));
    }
    if matches!(projection, ToolCatalogProjection::ReadOnly) && !tool.read_only {
        return Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de escritura no está disponible en modo de solo lectura.",
            false,
        ));
    }
    if matches!(projection, ToolCatalogProjection::PublishedTaskManager) && !is_published_tool(tool) {
        return Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta no está disponible en la publicación de Task Manager.",
            false,
        ));
    }

    match tool_policy(&tool.name) {
        ToolPolicy::Public | ToolPolicy::LibraryRead | ToolPolicy::LibraryWrite => Ok(()),
        ToolPolicy::Memory
            if context.actor.is_library_owner() && context.persistence_policy.allows_memory() =>
        {
            Ok(())
        }
        ToolPolicy::Memory => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta no está autorizada para este usuario.",
            false,
        )),
        // Mail and calendar are confidential: the owner or whoever may see
        // #Confidencial.
        ToolPolicy::Mail if principal.can_access_confidential_context() => Ok(()),
        ToolPolicy::Mail => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "El correo y el calendario requieren autorización del contexto confidencial.",
            false,
        )),
        ToolPolicy::TaskRead | ToolPolicy::TaskWrite
            if matches!(
                context.scope,
                BackendScope::TaskManager | BackendScope::Library
            ) =>
        {
            Ok(())
        }
        ToolPolicy::TaskRead | ToolPolicy::TaskWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta no está autorizada para este scope.",
            false,
        )),
        // Routine data is always scoped to the acting library user, so any
        // authorized user may read and write their own routine.
        ToolPolicy::RoutineRead | ToolPolicy::RoutineWrite
            if matches!(context.scope, BackendScope::Library | BackendScope::Finance) =>
        {
            Ok(())
        }
        ToolPolicy::RoutineRead | ToolPolicy::RoutineWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de Rutina no está autorizada para este scope.",
            false,
        )),
        // Agenda data is scoped to the acting library user, as Rutina.
        ToolPolicy::AgendaRead | ToolPolicy::AgendaWrite
            if matches!(context.scope, BackendScope::Library | BackendScope::Finance) =>
        {
            Ok(())
        }
        ToolPolicy::AgendaRead | ToolPolicy::AgendaWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de Agenda no está autorizada para este scope.",
            false,
        )),
        // Salud data is always the acting user's own, as Rutina.
        ToolPolicy::HealthRead | ToolPolicy::HealthWrite if matches!(context.scope, BackendScope::Library | BackendScope::Finance) => Ok(()),
        ToolPolicy::HealthRead | ToolPolicy::HealthWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de Salud no está autorizada para este scope.",
            false,
        )),
        // Gimnasio: the acting user's routines and workouts, and the library's catalog.
        ToolPolicy::GymRead | ToolPolicy::GymWrite if matches!(context.scope, BackendScope::Library | BackendScope::Finance) => Ok(()),
        ToolPolicy::GymRead | ToolPolicy::GymWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de Gimnasio no está autorizada para este scope.",
            false,
        )),
        // Recipes are library files: whoever uses the library or finance chat.
        ToolPolicy::RecipeRead | ToolPolicy::RecipeWrite if matches!(context.scope, BackendScope::Library | BackendScope::Finance) => Ok(()),
        ToolPolicy::RecipeRead | ToolPolicy::RecipeWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta de Recetas no está autorizada para este scope.",
            false,
        )),
        // The images run on the Owner's GPU and go with a reply they read:
        // the app chats and Telegram, never a published board nor Meeting.
        ToolPolicy::Images
            if context.actor.is_library_owner()
                && matches!(context.channel, BackendChannel::App | BackendChannel::Telegram) =>
        {
            Ok(())
        }
        ToolPolicy::Images => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Las imágenes las genera solo el Owner, en los chats de la app o en Telegram.",
            false,
        )),
        // The terminal, the browser and WhatsApp are the host PC's: only
        // the Owner, from the app chats (an AI task runs as one) or their
        // Telegram. A published board, Meeting and any other user never
        // reach them, whoever logs in there.
        ToolPolicy::Terminal | ToolPolicy::Browser | ToolPolicy::WhatsApp | ToolPolicy::Scripts
            if context.actor.is_library_owner()
                && matches!(context.channel, BackendChannel::App | BackendChannel::Telegram)
                && !matches!(projection, ToolCatalogProjection::PublishedTaskManager)
                && context.persistence_policy != super::context::PersistencePolicy::PublishedNoMemory =>
        {
            Ok(())
        }
        ToolPolicy::Terminal => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La terminal de la PC host la usa solo el Owner, desde los chats de la app o su Telegram.",
            false,
        )),
        ToolPolicy::Browser => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "El navegador de la PC host lo usa solo el Owner, desde los chats de la app o su Telegram.",
            false,
        )),
        ToolPolicy::WhatsApp => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "El WhatsApp vinculado lo usa solo el Owner, desde los chats de la app o su Telegram.",
            false,
        )),
        ToolPolicy::Scripts => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Los scripts de soporte corren solo para el Owner, desde los chats de la app o su Telegram.",
            false,
        )),
        // The actions answer only the Owner, by Telegram.
        ToolPolicy::AiActionRead | ToolPolicy::AiActionWrite if context.actor.is_library_owner() => Ok(()),
        ToolPolicy::AiActionRead | ToolPolicy::AiActionWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Las acciones de IA son solo del Owner de la biblioteca.",
            false,
        )),
        // The tasks keep the Owner's memory and answer the Owner.
        ToolPolicy::AiTaskRead | ToolPolicy::AiTaskWrite
            if context.actor.is_library_owner() && context.persistence_policy.allows_memory() =>
        {
            Ok(())
        }
        ToolPolicy::AiTaskRead | ToolPolicy::AiTaskWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Las Tareas IA son solo del Owner de la biblioteca.",
            false,
        )),
        // The meeting is recorded on the Owner's device: only the Owner's
        // library chats reach it.
        ToolPolicy::MeetingRead | ToolPolicy::MeetingWrite
            if context.actor.is_library_owner() && context.scope == BackendScope::Library =>
        {
            Ok(())
        }
        ToolPolicy::MeetingRead | ToolPolicy::MeetingWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Las herramientas de la reunión son solo del Owner.",
            false,
        )),
        ToolPolicy::FinanceRead | ToolPolicy::FinanceWrite
            if principal.can_access_confidential_context()
                && matches!(context.scope, BackendScope::Finance | BackendScope::Library) =>
        {
            Ok(())
        }
        ToolPolicy::FinanceRead | ToolPolicy::FinanceWrite => Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "La herramienta financiera requiere autorización del contexto confidencial.",
            false,
        )),
    }
}

pub fn project_tool_catalog(
    context: &BackendRequestContext,
    principal: &AuthorizationPrincipal,
    tools: &[ToolDefinition],
    projection: ToolCatalogProjection,
) -> Result<Vec<ToolDefinition>, BackendError> {
    let mut names = HashSet::new();
    let mut projected = Vec::new();
    for tool in tools {
        if !names.insert(tool.name.clone()) {
            return Err(BackendError::invalid_input(
                "El catálogo contiene nombres de tools duplicados.",
            ));
        }
        if authorize_tool_call(context, principal, tool, projection).is_ok() {
            projected.push(tool.clone());
        }
    }
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_documents_a_catalog_tool_with_an_object_schema() {
        let catalog = canonical_tool_catalog();
        for (name, entry) in tool_schemas() {
            assert!(
                catalog.iter().any(|tool| &tool.name == name),
                "schema without tool: {name}"
            );
            let parameters = &entry["parameters"];
            assert_eq!(parameters["type"], "object", "{name}");
            assert!(parameters["properties"].is_object(), "{name}");
            assert!(!entry["description"].as_str().unwrap_or_default().is_empty(), "{name}");
        }
    }

    #[test]
    fn a_chat_keeps_all_tools_only_those_that_read_or_none_but_memory() {
        let tools = || {
            canonical_tool_catalog()
                .into_iter()
                .filter(|tool| ["read_library_documents", "create_library_note", "add_agent_memory"].contains(&tool.name.as_str()))
                .collect::<Vec<_>>()
        };
        let names = |tools: Vec<ToolDefinition>| tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert_eq!(names(restrict_tool_access(tools(), ToolAccess::All, true)).len(), tools().len());
        let read_only = names(restrict_tool_access(tools(), ToolAccess::ReadOnly, true));
        assert!(read_only.contains(&"read_library_documents".to_string()));
        assert!(!read_only.contains(&"create_library_note".to_string()));
        assert!(read_only.contains(&"add_agent_memory".to_string()));
        assert_eq!(names(restrict_tool_access(tools(), ToolAccess::None, true)), vec!["add_agent_memory".to_string()]);
        let without_search = names(restrict_tool_access(tools(), ToolAccess::All, false));
        assert!(!without_search.contains(&"read_library_documents".to_string()));
        assert!(without_search.contains(&"create_library_note".to_string()));
    }

    #[test]
    fn a_turn_without_library_search_fits_the_tool_limit() {
        // Before, the turn listed every other tool and the catalog outgrew
        // the limit of the request ("El request supera el límite de tools").
        let catalog = canonical_tool_catalog();
        let limits = crate::protocol::BackendLimits::default();
        assert!(catalog.len() > limits.max_tools);
        let kept = restrict_tool_access(catalog, ToolAccess::All, false);
        assert!(kept.iter().all(|tool| tool_policy(&tool.name) != ToolPolicy::LibraryRead));
    }

    #[test]
    fn schemas_replace_generic_catalog_metadata() {
        let catalog = canonical_tool_catalog();
        let tool = catalog
            .iter()
            .find(|tool| tool.name == "create_finance_transaction")
            .expect("tool");
        assert!(tool.input_schema["properties"]["amount"].is_object());
        assert!(tool.requires_confirmation);
        assert!(catalog.iter().any(|tool| tool.name == "set_task_execution_plan"));
    }

    fn context(scope: BackendScope) -> BackendRequestContext {
        BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: super::super::BackendActor {
                library_user_id: "user-owner".into(),
                external_identity: None,
            },
            channel: super::super::BackendChannel::App,
            scope,
            persistence_policy: super::super::PersistencePolicy::Persistent,
        }
    }

    fn principal() -> AuthorizationPrincipal {
        AuthorizationPrincipal {
            library_id: "library-1".into(),
            library_user_id: "user-owner".into(),
            allowed_contexts: vec![CONFIDENTIAL_CONTEXT.into()],
            all_contexts: false,
        }
    }

    fn tool(name: &str, scope: BackendScope, read_only: bool) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: "test".into(),
            input_schema: serde_json::json!({"type": "object"}),
            scopes: vec![scope],
            read_only,
            requires_confirmation: !read_only,
        }
    }

    #[test]
    fn meeting_tools_are_the_owners_and_stay_out_of_published_boards() {
        let tools = project_tool_catalog(&context(BackendScope::Library), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        for name in crate::meeting_tools::MEETING_READ_TOOLS.iter().chain(crate::meeting_tools::MEETING_WRITE_TOOLS.iter()) {
            let tool = tools.iter().find(|tool| tool.name == *name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.requires_confirmation, !tool.read_only, "{name}");
        }
        assert_eq!(tool_policy("correct_meeting_transcript"), ToolPolicy::MeetingWrite);
        assert_eq!(tool_policy("get_meeting_overview"), ToolPolicy::MeetingRead);
        assert_eq!(crate::tool_routing::tool_area("update_meeting_notes"), Some(crate::tool_routing::ToolArea::Meeting));
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), allowed_contexts: Vec::new(), ..principal() };
        let guest_tools = project_tool_catalog(&guest, &guest_principal, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("guest");
        assert!(guest_tools.iter().all(|tool| !crate::meeting_tools::is_meeting_tool(&tool.name)));
        let board = project_tool_catalog(&context(BackendScope::TaskManager), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::PublishedTaskManager);
        if let Ok(board) = board {
            assert!(board.iter().all(|tool| !crate::meeting_tools::is_meeting_tool(&tool.name)));
        }
    }

    #[test]
    fn the_terminal_is_the_owners_in_the_app_and_telegram_and_never_published() {
        let has_terminal = |context: &BackendRequestContext, principal: &AuthorizationPrincipal, projection| {
            project_tool_catalog(context, principal, &canonical_tool_catalog(), projection)
                .map(|tools| tools.iter().any(|tool| crate::terminal::is_terminal_tool(&tool.name)))
                .unwrap_or(false)
        };
        let terminal = canonical_tool_catalog().into_iter().find(|tool| crate::terminal::is_terminal_tool(&tool.name)).expect("terminal");
        assert!(terminal.requires_confirmation && !terminal.read_only);
        assert_eq!(crate::tool_routing::tool_area(&terminal.name), None);
        for scope in [BackendScope::Library, BackendScope::Document, BackendScope::TaskManager] {
            assert!(has_terminal(&context(scope.clone()), &principal(), ToolCatalogProjection::Full), "{scope:?}");
        }
        let mut telegram = context(BackendScope::Library);
        telegram.channel = BackendChannel::Telegram;
        telegram.actor.external_identity =
            Some(crate::context::ExternalIdentity { provider: "telegram".into(), user_id: "1".into(), chat_id: Some(1) });
        assert!(has_terminal(&telegram, &principal(), ToolCatalogProjection::Full));
        // Another user, even with every context, never gets it.
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), all_contexts: true, ..principal() };
        assert!(!has_terminal(&guest, &guest_principal, ToolCatalogProjection::Full));
        // The published board, even with the Owner logged in, and Meeting.
        let mut published = context(BackendScope::TaskManager);
        published.channel = BackendChannel::Published;
        published.persistence_policy = super::super::PersistencePolicy::PublishedNoMemory;
        assert!(!has_terminal(&published, &principal(), ToolCatalogProjection::PublishedTaskManager));
        assert!(!has_terminal(&published, &principal(), ToolCatalogProjection::Full));
        assert!(!has_terminal(&context(BackendScope::TaskManager), &principal(), ToolCatalogProjection::PublishedTaskManager));
        let mut meeting = context(BackendScope::Library);
        meeting.channel = BackendChannel::Meeting;
        assert!(!has_terminal(&meeting, &principal(), ToolCatalogProjection::Full));
        // Read-only chats and runs (the hourly review) leave it out.
        let read_only = restrict_tool_access(vec![terminal], ToolAccess::ReadOnly, true);
        assert!(read_only.is_empty());
    }

    #[test]
    fn the_browser_is_the_owners_where_the_terminal_is_and_its_clicks_ask_first() {
        let browser_tools = |context: &BackendRequestContext, principal: &AuthorizationPrincipal, projection| {
            project_tool_catalog(context, principal, &canonical_tool_catalog(), projection)
                .map(|tools| tools.into_iter().filter(|tool| crate::browser::is_browser_tool(&tool.name)).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let owner = browser_tools(&context(BackendScope::Library), &principal(), ToolCatalogProjection::Full);
        assert_eq!(owner.len(), crate::browser::BROWSER_TOOLS.len());
        for tool in &owner {
            assert!(!tool.read_only, "{}", tool.name);
            assert_eq!(tool.requires_confirmation, crate::browser::is_browser_interaction_tool(&tool.name), "{}", tool.name);
            assert!(tool_schemas().contains_key(&tool.name), "{} has no schema", tool.name);
            assert_eq!(crate::tool_routing::tool_area(&tool.name), Some(crate::tool_routing::ToolArea::Browser), "{}", tool.name);
        }
        let mut telegram = context(BackendScope::Library);
        telegram.channel = BackendChannel::Telegram;
        telegram.actor.external_identity =
            Some(crate::context::ExternalIdentity { provider: "telegram".into(), user_id: "1".into(), chat_id: Some(1) });
        assert_eq!(browser_tools(&telegram, &principal(), ToolCatalogProjection::Full).len(), owner.len());
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), all_contexts: true, ..principal() };
        assert!(browser_tools(&guest, &guest_principal, ToolCatalogProjection::Full).is_empty());
        let mut published = context(BackendScope::TaskManager);
        published.channel = BackendChannel::Published;
        published.persistence_policy = super::super::PersistencePolicy::PublishedNoMemory;
        assert!(browser_tools(&published, &principal(), ToolCatalogProjection::PublishedTaskManager).is_empty());
        assert!(browser_tools(&published, &principal(), ToolCatalogProjection::Full).is_empty());
        let mut meeting = context(BackendScope::Library);
        meeting.channel = BackendChannel::Meeting;
        assert!(browser_tools(&meeting, &principal(), ToolCatalogProjection::Full).is_empty());
        assert!(restrict_tool_access(owner, ToolAccess::ReadOnly, true).is_empty());
    }

    #[test]
    fn whatsapp_is_the_owners_where_the_terminal_is_and_sending_asks_first() {
        let whatsapp_tools = |context: &BackendRequestContext, principal: &AuthorizationPrincipal, projection| {
            project_tool_catalog(context, principal, &canonical_tool_catalog(), projection)
                .map(|tools| tools.into_iter().filter(|tool| crate::whatsapp::is_whatsapp_tool(&tool.name)).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let owner = whatsapp_tools(&context(BackendScope::Library), &principal(), ToolCatalogProjection::Full);
        assert_eq!(owner.len(), crate::whatsapp::WHATSAPP_TOOLS.len());
        for tool in &owner {
            let sends = crate::whatsapp::is_whatsapp_send_tool(&tool.name);
            assert_eq!(tool.read_only, !sends, "{}", tool.name);
            assert_eq!(tool.requires_confirmation, sends, "{}", tool.name);
            assert!(tool_schemas().contains_key(&tool.name), "{} has no schema", tool.name);
            assert_eq!(crate::tool_routing::tool_area(&tool.name), Some(crate::tool_routing::ToolArea::WhatsApp), "{}", tool.name);
        }
        let mut telegram = context(BackendScope::Library);
        telegram.channel = BackendChannel::Telegram;
        telegram.actor.external_identity =
            Some(crate::context::ExternalIdentity { provider: "telegram".into(), user_id: "1".into(), chat_id: Some(1) });
        assert_eq!(whatsapp_tools(&telegram, &principal(), ToolCatalogProjection::Full).len(), owner.len());
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), all_contexts: true, ..principal() };
        assert!(whatsapp_tools(&guest, &guest_principal, ToolCatalogProjection::Full).is_empty());
        let mut published = context(BackendScope::TaskManager);
        published.channel = BackendChannel::Published;
        published.persistence_policy = super::super::PersistencePolicy::PublishedNoMemory;
        assert!(whatsapp_tools(&published, &principal(), ToolCatalogProjection::PublishedTaskManager).is_empty());
        assert!(whatsapp_tools(&published, &principal(), ToolCatalogProjection::Full).is_empty());
        let mut meeting = context(BackendScope::Library);
        meeting.channel = BackendChannel::Meeting;
        assert!(whatsapp_tools(&meeting, &principal(), ToolCatalogProjection::Full).is_empty());
        // A read-only run (the hourly review) reads, never sends.
        let read_only = restrict_tool_access(owner, ToolAccess::ReadOnly, true);
        assert!(!read_only.is_empty() && read_only.iter().all(|tool| !crate::whatsapp::is_whatsapp_send_tool(&tool.name)));
    }

    #[test]
    fn the_support_scripts_are_the_owners_where_the_terminal_is_and_saving_asks_first() {
        let script_tools = |context: &BackendRequestContext, principal: &AuthorizationPrincipal, projection| {
            project_tool_catalog(context, principal, &canonical_tool_catalog(), projection)
                .map(|tools| tools.into_iter().filter(|tool| crate::agent_scripts::is_script_tool(&tool.name)).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let owner = script_tools(&context(BackendScope::Library), &principal(), ToolCatalogProjection::Full);
        assert_eq!(owner.len(), crate::agent_scripts::SCRIPT_TOOLS.len());
        for tool in &owner {
            assert_eq!(tool.requires_confirmation, tool.name == crate::agent_scripts::SAVE_TOOL, "{}", tool.name);
            assert_eq!(tool.read_only, tool.name == crate::agent_scripts::LIST_TOOL, "{}", tool.name);
            assert!(tool_schemas().contains_key(&tool.name), "{} has no schema", tool.name);
            assert_eq!(crate::tool_routing::tool_area(&tool.name), Some(crate::tool_routing::ToolArea::Scripts), "{}", tool.name);
        }
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), all_contexts: true, ..principal() };
        assert!(script_tools(&guest, &guest_principal, ToolCatalogProjection::Full).is_empty());
        let mut published = context(BackendScope::TaskManager);
        published.channel = BackendChannel::Published;
        published.persistence_policy = super::super::PersistencePolicy::PublishedNoMemory;
        assert!(script_tools(&published, &principal(), ToolCatalogProjection::PublishedTaskManager).is_empty());
        assert!(script_tools(&published, &principal(), ToolCatalogProjection::Full).is_empty());
        let mut meeting = context(BackendScope::Library);
        meeting.channel = BackendChannel::Meeting;
        assert!(script_tools(&meeting, &principal(), ToolCatalogProjection::Full).is_empty());
        let read_only = restrict_tool_access(owner, ToolAccess::ReadOnly, true);
        assert_eq!(read_only.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(), [crate::agent_scripts::LIST_TOOL]);
    }

    #[test]
    fn health_tools_reach_every_user_for_their_own_data() {
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), allowed_contexts: Vec::new(), ..principal() };
        let tools = project_tool_catalog(&guest, &guest_principal, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        for name in crate::health::tools::HEALTH_READ_TOOLS.iter().chain(crate::health::tools::HEALTH_WRITE_TOOLS.iter()) {
            let tool = tools.iter().find(|tool| tool.name == *name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.requires_confirmation, !tool.read_only, "{name}");
        }
        assert_eq!(tool_policy("log_meal"), ToolPolicy::HealthWrite);
        assert_eq!(tool_policy("get_health_summary"), ToolPolicy::HealthRead);
        let finance = project_tool_catalog(&context(BackendScope::Finance), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("finance");
        assert!(finance.iter().any(|tool| tool.name == "log_meal"));
        let board = project_tool_catalog(&context(BackendScope::TaskManager), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("board");
        assert!(board.iter().all(|tool| !crate::health::tools::is_health_tool(&tool.name)));
        assert_eq!(crate::tool_routing::tool_area("log_weight"), Some(crate::tool_routing::ToolArea::Health));
    }

    #[test]
    fn gym_tools_reach_every_user_with_a_schema_and_an_area() {
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), allowed_contexts: Vec::new(), ..principal() };
        let tools = project_tool_catalog(&guest, &guest_principal, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        for name in crate::gym::tools::GYM_READ_TOOLS.iter().chain(crate::gym::tools::GYM_WRITE_TOOLS.iter()) {
            let tool = tools.iter().find(|tool| tool.name == *name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.requires_confirmation, !tool.read_only, "{name}");
            assert!(tool_schemas().contains_key(*name), "{name} has no schema");
            assert_eq!(crate::tool_routing::tool_area(name), Some(crate::tool_routing::ToolArea::Gym), "{name}");
        }
        assert_eq!(tool_policy("save_gym_routine"), ToolPolicy::GymWrite);
        assert_eq!(tool_policy("get_gym_summary"), ToolPolicy::GymRead);
        let board = project_tool_catalog(&context(BackendScope::TaskManager), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("board");
        assert!(board.iter().all(|tool| !crate::gym::tools::is_gym_tool(&tool.name)));
    }

    #[test]
    fn recipe_tools_reach_the_library_chat_of_every_user() {
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), allowed_contexts: Vec::new(), ..principal() };
        let tools = project_tool_catalog(&guest, &guest_principal, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        for name in ["list_recipes", "get_recipe", "create_recipe", "update_recipe", "delete_recipe"] {
            let tool = tools.iter().find(|tool| tool.name == name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool.requires_confirmation, !tool.read_only, "{name}");
        }
        let finance = project_tool_catalog(&context(BackendScope::Finance), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("finance");
        assert!(finance.iter().any(|tool| tool.name == "create_recipe"));
        assert_eq!(crate::tool_routing::tool_area("create_recipe"), Some(crate::tool_routing::ToolArea::Recipes));
    }

    #[test]
    fn ai_action_tools_reach_every_chat_for_the_owner_only() {
        let limits = crate::protocol::BackendLimits::default();
        let names = crate::ai_actions::tools::AI_ACTION_READ_TOOLS
            .iter()
            .chain(crate::ai_actions::tools::AI_ACTION_WRITE_TOOLS.iter())
            .collect::<Vec<_>>();
        for scope in [BackendScope::Library, BackendScope::Finance, BackendScope::TaskManager, BackendScope::Document, BackendScope::Graph] {
            let tools = project_tool_catalog(&context(scope.clone()), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
            for name in &names {
                let tool = tools.iter().find(|tool| &&tool.name.as_str() == name).unwrap_or_else(|| panic!("{name} in {scope:?}"));
                assert_eq!(tool.requires_confirmation, !tool.read_only, "{name}");
            }
            assert_every_area_fits(&restrict_tool_access(tools, ToolAccess::All, true), limits.max_tools);
        }
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-ana".into();
        let guest_principal = AuthorizationPrincipal { library_user_id: "user-ana".into(), ..principal() };
        let create = canonical_tool_catalog().into_iter().find(|tool| tool.name == "create_ai_action").expect("tool");
        assert_eq!(
            authorize_tool_call(&guest, &guest_principal, &create, ToolCatalogProjection::Full).expect_err("owner only").code,
            BackendErrorCode::Forbidden
        );
        let published = project_tool_catalog(&context(BackendScope::Library), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::PublishedTaskManager).expect("published");
        assert!(published.iter().all(|tool| !tool.name.contains("ai_action")));
        assert_eq!(crate::tool_routing::tool_area("create_ai_action"), Some(crate::tool_routing::ToolArea::Actions));
    }

    /// The hourly review asks for tasks, routine, agenda, mail and finance at
    /// once: together they pass the tool limit, so Finanzas used to be left
    /// out without a word while the agent was told it had it.
    #[test]
    fn the_hourly_review_reads_every_area_it_asks_for() {
        use crate::tool_routing::{area_coverage, turn_tools, AreaCoverage, ToolArea};
        let mut owner = principal();
        owner.all_contexts = true;
        let library = restrict_tool_access(
            project_tool_catalog(&context(BackendScope::Library), &owner, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog"),
            ToolAccess::All,
            true,
        );
        let max = crate::protocol::BackendLimits::default().max_tools;
        let areas = [ToolArea::Tasks, ToolArea::Routine, ToolArea::Agenda, ToolArea::Mail, ToolArea::Finance];
        let round = turn_tools(&library, &areas, max);
        assert!(round.len() <= max);
        let coverage = area_coverage(&library, &round, &areas);
        assert_eq!(coverage.full.first(), Some(&ToolArea::Tasks));
        assert!(coverage.missing.is_empty(), "{coverage:?}");
        assert_ne!(coverage, AreaCoverage { full: areas.to_vec(), ..AreaCoverage::default() });
        for name in ["get_finance_dashboard", "list_agenda", "list_gmail_messages", "get_task_board_summary"] {
            assert!(round.iter().any(|tool| tool.name == name), "{name}");
        }
    }

    /// Every chat is routed by area: each area it offers fits in one round
    /// with the tools every round keeps and `change_tool_areas`.
    fn assert_every_area_fits(tools: &[ToolDefinition], max_tools: usize) {
        use crate::tool_routing::{offered_areas, tool_area, turn_tools};
        for area in offered_areas(tools) {
            let round = turn_tools(tools, &[area], max_tools);
            assert!(round.len() <= max_tools, "{area:?}: {}", round.len());
            let all = tools.iter().filter(|tool| tool_area(&tool.name) == Some(area)).count();
            let kept = round.iter().filter(|tool| tool_area(&tool.name) == Some(area)).count();
            assert_eq!(kept, all, "{area:?} does not fit in a round");
        }
    }

    #[test]
    fn mail_tools_are_confidential_and_fit_the_tool_limit() {
        let mut owner = principal();
        owner.all_contexts = true;
        let limits = crate::protocol::BackendLimits::default();
        for scope in [BackendScope::Document, BackendScope::TaskManager] {
            let tools = project_tool_catalog(&context(scope.clone()), &owner, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
            assert!(tools.iter().any(|tool| tool.name == "send_gmail_message"), "{scope:?}");
            assert_every_area_fits(&restrict_tool_access(tools, ToolAccess::All, true), limits.max_tools);
        }
        // The library chat reaches every area and is routed: each area fits
        // in a turn with the tools every turn keeps.
        let library = restrict_tool_access(
            project_tool_catalog(&context(BackendScope::Library), &owner, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog"),
            ToolAccess::All,
            true,
        );
        assert!(crate::tool_routing::needs_routing(&library));
        let offered = crate::tool_routing::offered_areas(&library);
        assert!(offered.contains(&crate::tool_routing::ToolArea::Finance) && offered.contains(&crate::tool_routing::ToolArea::Mail));
        assert_every_area_fits(&library, limits.max_tools);
        let mut guest = context(BackendScope::Library);
        guest.actor.library_user_id = "user-guest".into();
        let mut confidential = principal();
        confidential.library_user_id = "user-guest".into();
        let tools = project_tool_catalog(&guest, &confidential, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        assert!(tools.iter().any(|tool| tool.name == "list_gmail_messages"));
        let mut personal_only = confidential.clone();
        personal_only.allowed_contexts = vec!["#Personal".into()];
        let tools = project_tool_catalog(&guest, &personal_only, &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        assert!(!tools.iter().any(|tool| tool_policy(&tool.name) == ToolPolicy::Mail));
        let graph = project_tool_catalog(&context(BackendScope::Graph), &owner, &canonical_tool_catalog(), ToolCatalogProjection::ReadOnly).expect("catalog");
        assert!(!graph.iter().any(|tool| tool_policy(&tool.name) == ToolPolicy::Mail));
    }

    #[test]
    fn the_library_chat_and_telegram_reach_every_task_manager_tool() {
        let library = project_tool_catalog(&context(BackendScope::Library), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
        let task_tools = super::super::task_manager_tools::task_manager_read_tool_contracts()
            .into_iter()
            .chain(super::super::task_manager_tools::task_manager_mutation_tool_contracts())
            .collect::<Vec<_>>();
        for tool in &task_tools {
            assert!(library.iter().any(|candidate| candidate.name == tool.name), "{}", tool.name);
        }
        assert!(crate::tool_routing::offered_areas(&library).contains(&crate::tool_routing::ToolArea::Tasks));
        for name in ["update_task_group", "reorder_task_groups"] {
            assert_eq!(tool_policy(name), ToolPolicy::TaskWrite, "{name}");
        }
        // Task tools stay out of the chats that are not about tasks.
        for scope in [BackendScope::Finance, BackendScope::Document, BackendScope::Graph] {
            let tools = project_tool_catalog(&context(scope.clone()), &principal(), &canonical_tool_catalog(), ToolCatalogProjection::Full).expect("catalog");
            assert!(!tools.iter().any(|tool| task_tools.iter().any(|task| task.name == tool.name)), "{scope:?}");
        }
    }

    #[test]
    fn filters_finance_tools_without_confidential_access() {
        let mut principal = principal();
        principal.allowed_contexts.clear();
        let tools = project_tool_catalog(
            &context(BackendScope::Finance),
            &principal,
            &[tool("get_finance_dashboard", BackendScope::Finance, true)],
            ToolCatalogProjection::Full,
        )
        .expect("catalog projects");
        assert!(tools.is_empty());
    }

    #[test]
    fn a_published_guest_gets_only_task_tools_and_questions() {
        let mut owner = principal();
        owner.all_contexts = true;
        for scope in [BackendScope::TaskManager, BackendScope::Library] {
            let tools = project_tool_catalog(&context(scope), &owner, &canonical_tool_catalog(), ToolCatalogProjection::PublishedTaskManager)
                .expect("catalog");
            assert!(tools.iter().any(|tool| tool.name == "change_task_state"));
            assert!(tools.iter().any(|tool| tool.name == "request_user_confirmation"));
            for tool in &tools {
                let task = crate::tool_routing::tool_area(&tool.name) == Some(crate::tool_routing::ToolArea::Tasks);
                assert!(task || tool.name.starts_with("request_user_"), "{} reached the publication", tool.name);
            }
        }
    }

    #[test]
    fn published_projection_keeps_only_task_scope() {
        let tools = project_tool_catalog(
            &context(BackendScope::TaskManager),
            &principal(),
            &[
                tool("read_task_tickets", BackendScope::TaskManager, true),
                tool("search_web", BackendScope::Library, true),
            ],
            ToolCatalogProjection::PublishedTaskManager,
        )
        .expect("catalog projects");
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["read_task_tickets"]
        );
    }

    #[test]
    fn routine_tools_are_library_scoped_and_confirm_writes() {
        let catalog = canonical_tool_catalog();
        let write = catalog
            .iter()
            .find(|tool| tool.name == "set_routine_completions")
            .expect("tool");
        assert!(write.requires_confirmation && !write.read_only);
        assert_eq!(tool_policy("get_routine_day"), ToolPolicy::RoutineRead);
        assert_eq!(tool_policy("list_agenda"), ToolPolicy::AgendaRead);
        assert_eq!(tool_policy("create_agenda_event"), ToolPolicy::AgendaWrite);
        assert!(write.scopes.contains(&BackendScope::Finance));
        assert!(authorize_tool_call(
            &context(BackendScope::TaskManager),
            &principal(),
            &tool("get_routine_day", BackendScope::TaskManager, true),
            ToolCatalogProjection::Full,
        )
        .is_err());
        let mut principal = principal();
        principal.allowed_contexts.clear();
        let tools = project_tool_catalog(
            &context(BackendScope::Library),
            &principal,
            &[tool("get_routine_day", BackendScope::Library, true)],
            ToolCatalogProjection::Full,
        )
        .expect("catalog projects");
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn memory_tools_need_the_owner_and_a_memory_policy() {
        let memory = tool("add_agent_memory", BackendScope::Library, false);
        assert!(authorize_tool_call(&context(BackendScope::Library), &principal(), &memory, ToolCatalogProjection::Full).is_ok());
        let note_chat_memory = canonical_tool_catalog().into_iter().find(|tool| tool.name == "add_agent_memory").expect("tool");
        assert!(authorize_tool_call(&context(BackendScope::Document), &principal(), &note_chat_memory, ToolCatalogProjection::Full).is_ok());
        assert!(authorize_tool_call(&context(BackendScope::Finance), &principal(), &note_chat_memory, ToolCatalogProjection::Full).is_err());
        let mut without_memory = context(BackendScope::Library);
        without_memory.persistence_policy = super::super::PersistencePolicy::EphemeralNoMemory;
        assert!(authorize_tool_call(&without_memory, &principal(), &memory, ToolCatalogProjection::Full).is_err());
        let thought = canonical_tool_catalog().into_iter().find(|tool| tool.name == "add_agent_thought").expect("tool");
        assert!(!thought.requires_confirmation);
        assert!(authorize_tool_call(&context(BackendScope::Library), &principal(), &thought, ToolCatalogProjection::Full).is_ok());
        assert!(authorize_tool_call(&without_memory, &principal(), &thought, ToolCatalogProjection::Full).is_err());
        assert_eq!(restrict_tool_access(vec![thought], ToolAccess::ReadOnly, false).len(), 1);
        for name in ["add_agent_biography", "add_agent_talk", "add_agent_person", "add_agent_improvement"] {
            let tool = canonical_tool_catalog().into_iter().find(|tool| tool.name == name).expect("tool");
            assert!(!tool.requires_confirmation && tool_policy(name) == ToolPolicy::Memory);
            assert!(authorize_tool_call(&context(BackendScope::Library), &principal(), &tool, ToolCatalogProjection::Full).is_ok());
            assert!(authorize_tool_call(&without_memory, &principal(), &tool, ToolCatalogProjection::Full).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_tool_names() {
        let result = project_tool_catalog(
            &context(BackendScope::Library),
            &principal(),
            &[
                tool("read_task_tickets", BackendScope::Library, true),
                tool("read_task_tickets", BackendScope::Library, true),
            ],
            ToolCatalogProjection::Full,
        );
        assert_eq!(
            result.expect_err("duplicates are invalid").code,
            BackendErrorCode::InvalidInput
        );
    }
}

