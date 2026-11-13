//! Changes the agent saves without asking, by module: the person's own
//! everyday records (a meal, water, the weight, a set at the gym, a habit
//! done) when the library turned on «Registrar sin confirmar» for that
//! module (`ia.autoConfirmAreas`). Deleting, settings (profile, plan,
//! training routines, equipment, exercise sheets, habits themselves) and
//! every other module keep asking, and so does any run nobody is watching:
//! an AI action, an AI task, a webhook call or Notia's own runs.
//!
//! A change saved this way takes the path of «Confirmar todos»: its input
//! is checked with the same preview, it runs as confirmed and the agent
//! tells what it saved.

use crate::context::BackendChannel;
use crate::protocol::AgentRequest;
use crate::tool_routing::ToolArea;

/// Modules a library may save without asking, in the order they are shown.
pub const AUTO_CONFIRM_AREAS: [ToolArea; 3] = [ToolArea::Health, ToolArea::Gym, ToolArea::Routine];

/// Records of Salud: a meal (and correcting it), water, weight, steps,
/// sleep and a scale measurement. Not the profile, goal or plan.
const HEALTH_RECORDS: [&str; 7] =
    ["log_meal", "update_meal", "log_water", "log_weight", "log_steps", "log_sleep", "save_body_measurement"];
/// Records of Gimnasio: the training under way (start, sets, rest, finish)
/// and a training already done. Not routines, equipment or exercises.
const GYM_RECORDS: [&str; 2] = ["control_gym_session", "log_gym_workout"];
/// Records of Rutina: a task or habit of the day done or undone.
const ROUTINE_RECORDS: [&str; 2] = ["set_routine_task_status", "set_routine_completions"];

/// The tools of `area` that may save without asking.
pub fn records_of(area: ToolArea) -> &'static [&'static str] {
    match area {
        ToolArea::Health => &HEALTH_RECORDS,
        ToolArea::Gym => &GYM_RECORDS,
        ToolArea::Routine => &ROUTINE_RECORDS,
        _ => &[],
    }
}

/// The modules among `ids` that may save without asking: known ones, once
/// each, in `AUTO_CONFIRM_AREAS` order.
pub fn normalize_area_ids<S: AsRef<str>>(ids: &[S]) -> Vec<&'static str> {
    AUTO_CONFIRM_AREAS
        .into_iter()
        .filter(|area| ids.iter().any(|id| ToolArea::parse(id.as_ref()) == Some(*area)))
        .map(ToolArea::id)
        .collect()
}

/// The records a run saves without asking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoConfirm {
    areas: Vec<ToolArea>,
}

impl AutoConfirm {
    /// The library's choice (`area_ids`) for `request`: nothing for a run
    /// nobody is watching or outside the app and Telegram.
    pub fn for_request<S: AsRef<str>>(area_ids: &[S], request: &AgentRequest) -> Self {
        let asked_by_a_person = matches!(request.context.channel, BackendChannel::App | BackendChannel::Telegram)
            && !request.autonomous
            && request.scheduled_action.is_none()
            && request.webhook_call.is_none()
            && request.ai_task.is_none();
        if !asked_by_a_person {
            return Self::default();
        }
        let areas = normalize_area_ids(area_ids).into_iter().filter_map(ToolArea::parse).collect();
        Self { areas }
    }

    /// Whether some module saves its records without asking. Then the
    /// model's own asks (a plan, `request_user_confirmation`) do not stop a
    /// run either: each change that is not a record still asks in its gate.
    pub fn is_on(&self) -> bool {
        !self.areas.is_empty()
    }

    /// Whether the call of `tool_name` saves without asking.
    pub fn covers(&self, tool_name: &str) -> bool {
        self.areas.iter().any(|area| records_of(*area).contains(&tool_name))
    }

    /// What the model must know: without it, it asks on its own before a
    /// record (in its text, with `request_user_confirmation` or with a plan
    /// for two or more changes), and the person confirms anyway.
    pub fn guidance(&self) -> Option<String> {
        if self.areas.is_empty() {
            return None;
        }
        let modules = self.areas.iter().map(|area| area_label(*area)).collect::<Vec<_>>().join(", ");
        let tools = self.areas.iter().flat_map(|area| records_of(*area).iter().copied()).collect::<Vec<_>>().join(", ");
        Some(format!(
            "«Registrar sin confirmar» está activo en esta biblioteca para {modules}: {tools} se guardan apenas los llamás, sin que la persona confirme. Cuando pida uno de esos registros, llamá la herramienta enseguida: no preguntes si querés registrarlo, no uses request_user_confirmation y no armes un plan, aunque sean varios registros en el mismo mensaje (llamalas una tras otra). Si el mensaje mezcla registros con otros cambios, el plan es solo para los otros cambios. Si falta un dato sin el cual no se puede registrar, pedilo con request_user_clarification. Al terminar, contá en una línea qué registraste."
        ))
    }
}

fn area_label(area: ToolArea) -> &'static str {
    match area {
        ToolArea::Health => "Salud",
        ToolArea::Gym => "Gimnasio",
        ToolArea::Routine => "Rutina",
        _ => "otros módulos",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{tool_policy, ToolPolicy};

    /// The model is told which records save by themselves and not to ask,
    /// plan or confirm for them; with nothing on, it is told nothing.
    #[test]
    fn the_model_is_told_not_to_ask_for_records_that_save_by_themselves() {
        assert_eq!(AutoConfirm::default().guidance(), None);
        let on = AutoConfirm { areas: vec![ToolArea::Health, ToolArea::Gym] };
        let text = on.guidance().expect("guidance");
        assert!(text.contains("Salud, Gimnasio"), "{text}");
        assert!(text.contains("log_water") && text.contains("log_gym_workout"), "{text}");
        assert!(!text.contains("set_routine_task_status"), "{text}");
        assert!(text.contains("no uses request_user_confirmation") && text.contains("no armes un plan"), "{text}");
    }

    #[test]
    fn only_records_of_the_three_modules_are_offered_and_none_deletes() {
        for area in AUTO_CONFIRM_AREAS {
            for name in records_of(area) {
                assert_eq!(crate::tool_routing::tool_area(name), Some(area), "{name}");
                assert!(
                    matches!(tool_policy(name), ToolPolicy::HealthWrite | ToolPolicy::GymWrite | ToolPolicy::RoutineWrite),
                    "{name} is a record that writes"
                );
                assert!(!name.starts_with("delete_"), "{name}");
            }
        }
        for settings in ["save_health_profile", "set_health_plan", "set_weight_goal", "save_gym_routine", "set_gym_equipment", "save_gym_exercise", "save_routine", "save_routine_task", "delete_health_record", "delete_gym_workout", "delete_routine_task"] {
            assert!(AUTO_CONFIRM_AREAS.iter().all(|area| !records_of(*area).contains(&settings)), "{settings}");
        }
        assert!(records_of(ToolArea::Finance).is_empty());
    }

    #[test]
    fn the_library_choice_is_normalized() {
        assert_eq!(normalize_area_ids(&["rutina", " Salud ", "finanzas", "salud", "x"]), vec!["salud", "rutina"]);
        assert!(normalize_area_ids::<&str>(&[]).is_empty());
    }
}
