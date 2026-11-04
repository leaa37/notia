//! Python support scripts of the agent, in `.agent/scripts` of the library:
//! the agent writes one when a hard or repetitive job has no tool of its
//! own (or a script does it better than many steps), and the person may
//! drop their own there. Each script works as one more tool: the model
//! reads the list in the description of [`RUN_TOOL`] and runs a script with
//! its arguments. This module reads a script's header (its description and
//! parameters), writes the file the agent saves, checks the arguments and
//! writes what the model reads; the adapter finds Python on the host PC and
//! runs the scripts there.
//!
//! A script is `<name>.py` and starts with a docstring like this one:
//!
//! ```text
//! """
//! ---
//! description: Convierte un CSV de la biblioteca en una tabla Markdown.
//! parameters:
//!   - path (string, requerido): ruta del CSV dentro de la biblioteca
//!   - delimiter (string): separador; por defecto ","
//! ---
//! """
//! ```
//!
//! It gets its arguments as a JSON object on stdin, runs in the library's
//! folder (also in `NOTIA_LIBRARY`) and prints its result on stdout, as
//! JSON when it can; a non-zero exit code is a failure explained on stderr.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::error::BackendError;

pub const SCRIPTS_DIRECTORY: &str = ".agent/scripts";
/// The folder's guide for the person, written when it is missing.
pub const README_PATH: &str = ".agent/scripts/README.md";
pub const README: &str = include_str!("defaults/agent_scripts_readme.md");

pub const LIST_TOOL: &str = "list_agent_scripts";
pub const RUN_TOOL: &str = "run_agent_script";
pub const SAVE_TOOL: &str = "save_agent_script";
pub const SCRIPT_TOOLS: [&str; 3] = [LIST_TOOL, RUN_TOOL, SAVE_TOOL];

pub fn is_script_tool(name: &str) -> bool {
    SCRIPT_TOOLS.contains(&name)
}

/// Scripts read from the folder; the rest are left out.
pub const MAX_SCRIPTS: usize = 100;
pub const MAX_SCRIPT_BYTES: usize = 200 * 1024;
const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_PARAMETERS: usize = 20;
const MAX_PARAMETER_DESCRIPTION_CHARS: usize = 300;
const MAX_ARGUMENTS_BYTES: usize = 100 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Characters of the output and of the errors the model reads (the end).
const MAX_OUTPUT_CHARS: usize = 20_000;
const MAX_ERROR_CHARS: usize = 4_000;
/// Scripts the description of [`RUN_TOOL`] lists; `list_agent_scripts`
/// has them all.
const MAX_DESCRIBED_SCRIPTS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterKind {
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
}

impl ParameterKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_lowercase().as_str() {
            "string" | "str" | "texto" | "text" => Self::String,
            "number" | "float" | "numero" | "número" => Self::Number,
            "integer" | "int" | "entero" => Self::Integer,
            "boolean" | "bool" | "booleano" => Self::Boolean,
            "array" | "list" | "lista" => Self::Array,
            "object" | "dict" | "objeto" => Self::Object,
            _ => return None,
        })
    }

    fn id(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Array => "array",
            Self::Object => "object",
        }
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Number => value.is_number(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Boolean => value.is_boolean(),
            Self::Array => value.is_array(),
            Self::Object => value.is_object(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ParameterKind,
    pub required: bool,
    pub description: String,
}

/// A script of the folder: its name (the file without `.py`), its file
/// and what its header says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentScript {
    pub name: String,
    pub file: String,
    pub description: String,
    pub parameters: Vec<ScriptParameter>,
}

/// A script name: lower case letters, digits and `_`, starting with a
/// letter (it is also the file name).
pub fn validate_name(name: &str) -> Result<String, BackendError> {
    let name = name.trim().trim_end_matches(".py").to_string();
    let valid = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name.starts_with(|character: char| character.is_ascii_lowercase())
        && name.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_');
    if valid {
        Ok(name)
    } else {
        Err(BackendError::invalid_input(format!(
            "El nombre del script va en minúsculas, con letras, números y _, empezando con una letra y de hasta {MAX_NAME_CHARS} caracteres (por ejemplo csv_a_tabla)."
        )))
    }
}

pub fn script_path(name: &str) -> String {
    format!("{SCRIPTS_DIRECTORY}/{name}.py")
}

fn one_line(text: &str, max: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// The docstring a script starts with (after its `#` comment lines), and
/// where the code after it starts.
fn leading_docstring(content: &str) -> Option<(&str, usize)> {
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            offset += line.len();
            continue;
        }
        let rest = &content[offset..];
        let rest = rest.trim_start();
        let start = content.len() - rest.len();
        let rest = rest.strip_prefix(['r', 'R']).unwrap_or(rest);
        let skipped = content.len() - start - rest.len();
        let quote = ["\"\"\"", "'''"].into_iter().find(|quote| rest.starts_with(quote))?;
        let body = &rest[quote.len()..];
        let end = body.find(quote)?;
        let after = start + skipped + quote.len() + end + quote.len();
        return Some((&body[..end], after));
    }
    None
}

/// `- path (string, requerido): ruta del CSV`
fn parse_parameter(line: &str) -> Option<ScriptParameter> {
    let line = line.trim().strip_prefix('-')?.trim();
    let (head, description) = line.split_once(':').map_or((line, ""), |(head, description)| (head, description));
    let (name, details) = match head.split_once('(') {
        Some((name, details)) => (name.trim(), details.trim_end().trim_end_matches(')')),
        None => (head.trim(), ""),
    };
    let valid_name = !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !valid_name {
        return None;
    }
    let mut kind = ParameterKind::String;
    let mut required = false;
    for token in details.split(',').map(|token| token.trim().to_lowercase()) {
        match token.as_str() {
            "requerido" | "required" | "obligatorio" => required = true,
            "opcional" | "optional" | "" => {}
            other => kind = ParameterKind::parse(other).unwrap_or(kind),
        }
    }
    Some(ScriptParameter { name: name.to_string(), kind, required, description: one_line(description, MAX_PARAMETER_DESCRIPTION_CHARS) })
}

/// Reads a script of the folder; `None` when its file name is not a script
/// name. A script without a header still runs: its description is the
/// first line of its docstring, or empty.
pub fn parse_script(file: &str, content: &str) -> Option<AgentScript> {
    let name = validate_name(file.strip_suffix(".py")?).ok()?;
    let docstring = leading_docstring(content).map(|(docstring, _)| docstring.replace("\r\n", "\n")).unwrap_or_default();
    let mut description = String::new();
    let mut parameters = Vec::new();
    let trimmed = docstring.trim_start();
    match trimmed.strip_prefix("---\n").and_then(|header| header.find("\n---").map(|end| &header[..end])) {
        Some(header) => {
            let mut in_parameters = false;
            for line in header.lines() {
                if let Some(value) = line.strip_prefix("description:") {
                    description = one_line(value.trim().trim_matches(['"', '\'']), MAX_DESCRIPTION_CHARS);
                    in_parameters = false;
                } else if line.trim_end() == "parameters:" {
                    in_parameters = true;
                } else if in_parameters && line.trim_start().starts_with('-') {
                    if let Some(parameter) = parse_parameter(line) {
                        if parameters.len() < MAX_PARAMETERS && !parameters.iter().any(|known: &ScriptParameter| known.name == parameter.name) {
                            parameters.push(parameter);
                        }
                    }
                } else if !line.starts_with(' ') && !line.trim().is_empty() {
                    in_parameters = false;
                }
            }
        }
        None => description = one_line(trimmed.lines().find(|line| !line.trim().is_empty()).unwrap_or_default(), MAX_DESCRIPTION_CHARS),
    }
    Some(AgentScript { name, file: file.to_string(), description, parameters })
}

impl AgentScript {
    /// `csv_a_tabla(path*, delimiter)`, for the list in a description.
    fn signature(&self) -> String {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| format!("{}{}: {}", parameter.name, if parameter.required { "*" } else { "" }, parameter.kind.id()))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({parameters})", self.name)
    }

    /// The JSON schema of its arguments.
    pub fn parameters_schema(&self) -> Value {
        let properties = self
            .parameters
            .iter()
            .map(|parameter| (parameter.name.clone(), json!({ "type": parameter.kind.id(), "description": parameter.description })))
            .collect::<Map<_, _>>();
        let required = self.parameters.iter().filter(|parameter| parameter.required).map(|parameter| parameter.name.clone()).collect::<Vec<_>>();
        json!({ "type": "object", "properties": properties, "required": required })
    }

    /// The arguments must be an object with every required parameter, and
    /// the declared ones of their type; others pass as they are.
    pub fn check_arguments(&self, arguments: &Value) -> Result<(), BackendError> {
        let object = arguments.as_object().ok_or_else(|| BackendError::invalid_input("arguments tiene que ser un objeto JSON."))?;
        let missing = self.parameters.iter().filter(|parameter| parameter.required && object.get(&parameter.name).is_none_or(Value::is_null)).map(|parameter| parameter.name.as_str()).collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(BackendError::invalid_input(format!("Al script {} le faltan argumentos: {}.", self.name, missing.join(", "))));
        }
        for parameter in &self.parameters {
            if let Some(value) = object.get(&parameter.name).filter(|value| !value.is_null()) {
                if !parameter.kind.accepts(value) {
                    return Err(BackendError::invalid_input(format!("El argumento {} del script {} es de tipo {}.", parameter.name, self.name, parameter.kind.id())));
                }
            }
        }
        if serde_json::to_string(arguments).map_or(true, |text| text.len() > MAX_ARGUMENTS_BYTES) {
            return Err(BackendError::invalid_input("Los argumentos del script son demasiado grandes: pasale una ruta de archivo en lugar del contenido."));
        }
        Ok(())
    }
}

/// The description of [`RUN_TOOL`] with the scripts there are, so the
/// model sees them like tools.
pub fn run_tool_description(base: &str, scripts: &[AgentScript]) -> String {
    if scripts.is_empty() {
        return format!("{base}\n\nTodavía no hay scripts en .agent/scripts: creá uno con save_agent_script cuando haga falta.");
    }
    let mut lines = scripts
        .iter()
        .take(MAX_DESCRIBED_SCRIPTS)
        .map(|script| {
            let description = if script.description.is_empty() { "(sin descripción)" } else { script.description.as_str() };
            format!("- {}: {description}", script.signature())
        })
        .collect::<Vec<_>>();
    if scripts.len() > MAX_DESCRIBED_SCRIPTS {
        lines.push(format!("- … y {} más: list_agent_scripts los muestra todos.", scripts.len() - MAX_DESCRIBED_SCRIPTS));
    }
    format!("{base}\n\nScripts disponibles (nombre(parámetros; * = requerido): qué hace):\n{}", lines.join("\n"))
}

pub fn list_result(scripts: &[AgentScript], unreadable: &[String]) -> Value {
    let scripts = scripts
        .iter()
        .map(|script| json!({ "name": script.name, "file": script_path(&script.name), "description": script.description, "parameters": script.parameters_schema() }))
        .collect::<Vec<_>>();
    let mut result = json!({
        "scripts": scripts,
        "folder": SCRIPTS_DIRECTORY,
        "note": "Corrélos con run_agent_script. Reciben arguments como JSON por la entrada estándar, corren en la carpeta de la biblioteca y devuelven lo que imprimen.",
    });
    if !unreadable.is_empty() {
        if let Some(object) = result.as_object_mut() {
            object.insert("unreadable".into(), json!(unreadable));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RunRequest {
    pub script: String,
    pub arguments: Value,
    pub timeout: Duration,
}

impl RunRequest {
    pub fn from_arguments(arguments: &Value) -> Result<Self, BackendError> {
        let script = validate_name(arguments.get("script").and_then(Value::as_str).unwrap_or_default())?;
        let values = match arguments.get("arguments") {
            None | Some(Value::Null) => json!({}),
            // A model may send the object as a JSON string.
            Some(Value::String(text)) => serde_json::from_str::<Value>(text).map_err(|_| BackendError::invalid_input("arguments tiene que ser un objeto JSON."))?,
            Some(value) => value.clone(),
        };
        let timeout = match arguments.get("timeoutSeconds").and_then(Value::as_f64) {
            Some(seconds) if seconds >= 1.0 => Duration::from_secs_f64(seconds).min(MAX_TIMEOUT),
            Some(_) => return Err(BackendError::invalid_input("timeoutSeconds es de al menos 1.")),
            None => DEFAULT_TIMEOUT,
        };
        Ok(Self { script, arguments: values, timeout })
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Exited(Option<i32>),
    TimedOut,
}

fn tail(text: &str, max: usize) -> (String, bool) {
    let count = text.chars().count();
    if count <= max {
        (text.to_string(), false)
    } else {
        (format!("…{}", text.chars().skip(count - max).collect::<String>()), true)
    }
}

/// What the model reads after a run: the result when stdout is JSON, else
/// the output (its end), and stderr, the exit code and the time.
pub fn run_result(script: &str, end: RunEnd, elapsed: Duration, stdout: &str, stderr: &str) -> Value {
    let stdout = stdout.trim();
    let mut result = Map::new();
    result.insert("script".into(), json!(script));
    let (ok, status) = match end {
        RunEnd::Exited(Some(0)) => (true, "ok".to_string()),
        RunEnd::Exited(Some(code)) => (false, format!("terminó con el código {code}")),
        RunEnd::Exited(None) => (false, "terminó sin código de salida".to_string()),
        RunEnd::TimedOut => (false, "se cortó por tiempo".to_string()),
    };
    result.insert("ok".into(), json!(ok));
    result.insert("status".into(), json!(status));
    if let RunEnd::Exited(Some(code)) = end {
        result.insert("exitCode".into(), json!(code));
    }
    result.insert("seconds".into(), json!((elapsed.as_secs_f64() * 10.0).round() / 10.0));
    match serde_json::from_str::<Value>(stdout).ok().filter(|_| stdout.len() <= MAX_OUTPUT_CHARS * 4) {
        Some(value) => {
            result.insert("result".into(), value);
        }
        None if !stdout.is_empty() => {
            let (output, cut) = tail(stdout, MAX_OUTPUT_CHARS);
            result.insert("output".into(), json!(output));
            if cut {
                result.insert("outputCut".into(), json!("La salida era más larga: se muestra el final. Hacé que el script escriba lo largo a un archivo y devuelva un resumen."));
            }
        }
        None => {}
    }
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        result.insert("stderr".into(), json!(tail(stderr, MAX_ERROR_CHARS).0));
    }
    if !ok {
        result.insert("note".into(), json!("Si el error es del script, corregilo con save_agent_script y volvé a correrlo; si falta un paquete de Python, decíselo a la persona o instalalo con la terminal."));
    }
    Value::Object(result)
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ScriptParameter>,
    pub code: String,
}

impl SaveRequest {
    pub fn from_arguments(arguments: &Value) -> Result<Self, BackendError> {
        let name = validate_name(arguments.get("name").and_then(Value::as_str).unwrap_or_default())?;
        let description = one_line(arguments.get("description").and_then(Value::as_str).unwrap_or_default(), MAX_DESCRIPTION_CHARS);
        if description.is_empty() {
            return Err(BackendError::invalid_input("Falta description: qué hace el script, en una frase."));
        }
        let mut parameters = Vec::new();
        for parameter in arguments.get("parameters").and_then(Value::as_array).into_iter().flatten() {
            let field = |key: &str| parameter.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
            let name = field("name");
            if name.is_empty() || !name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_') {
                return Err(BackendError::invalid_input("Cada parámetro necesita name con letras, números o _."));
            }
            let kind = match field("type") {
                kind if kind.is_empty() => ParameterKind::String,
                kind => ParameterKind::parse(&kind).ok_or_else(|| BackendError::invalid_input(format!("El tipo «{kind}» no existe: usá string, number, integer, boolean, array u object.")))?,
            };
            if parameters.iter().any(|known: &ScriptParameter| known.name == name) {
                return Err(BackendError::invalid_input(format!("El parámetro {name} está repetido.")));
            }
            parameters.push(ScriptParameter {
                name,
                kind,
                required: parameter.get("required").and_then(Value::as_bool).unwrap_or(false),
                description: one_line(&field("description"), MAX_PARAMETER_DESCRIPTION_CHARS),
            });
        }
        if parameters.len() > MAX_PARAMETERS {
            return Err(BackendError::invalid_input(format!("Un script tiene hasta {MAX_PARAMETERS} parámetros.")));
        }
        let code = arguments.get("code").and_then(Value::as_str).unwrap_or_default().replace("\r\n", "\n");
        if code.trim().is_empty() {
            return Err(BackendError::invalid_input("Falta code: el código Python del script."));
        }
        let request = Self { name, description, parameters, code };
        if request.source().len() > MAX_SCRIPT_BYTES {
            return Err(BackendError::invalid_input("El script es demasiado largo: partilo en scripts más chicos."));
        }
        Ok(request)
    }

    /// The file: the header Notia reads, then the code without a header
    /// of its own.
    pub fn source(&self) -> String {
        let clean = |text: &str| text.replace("\"\"\"", "'''");
        let mut header = format!("\"\"\"\n---\ndescription: {}\n", clean(&self.description));
        if !self.parameters.is_empty() {
            header.push_str("parameters:\n");
            for parameter in &self.parameters {
                let required = if parameter.required { ", requerido" } else { "" };
                let description = if parameter.description.is_empty() { String::new() } else { format!(": {}", clean(&parameter.description)) };
                header.push_str(&format!("  - {} ({}{required}){description}\n", parameter.name, parameter.kind.id()));
            }
        }
        header.push_str("---\nScript de soporte de Notia: recibe los argumentos como un objeto JSON por la entrada\nestándar, corre en la carpeta de la biblioteca y escribe el resultado en la salida estándar.\n\"\"\"\n");
        let code = match leading_docstring(&self.code) {
            Some((docstring, after)) if docstring.trim_start().starts_with("---") => self.code[after..].trim_start_matches('\n'),
            _ => self.code.as_str(),
        };
        format!("{header}{}\n", code.trim_end())
    }

    /// The confirmation's title.
    pub fn summary(&self, exists: bool) -> String {
        if exists {
            format!("Cambiar el script de soporte {}", self.name)
        } else {
            format!("Crear el script de soporte {}", self.name)
        }
    }

    /// The confirmation's detail: where it goes and the whole file.
    pub fn detail(&self) -> String {
        format!("{}\n\n{}", script_path(&self.name), self.source())
    }
}

pub fn saved_result(request: &SaveRequest, replaced: bool) -> Value {
    json!({
        "saved": true,
        "replaced": replaced,
        "script": request.name,
        "file": script_path(&request.name),
        "note": "Ya se puede correr con run_agent_script.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n\"\"\"\n---\ndescription: Convierte un CSV en una tabla Markdown.\nparameters:\n  - path (string, requerido): ruta del CSV\n  - delimiter (texto): separador; por defecto \",\"\n  - max_rows (entero)\n---\nTexto libre.\n\"\"\"\nimport json, sys\nprint(json.dumps({\"ok\": True}))\n";

    #[test]
    fn a_script_header_gives_its_description_and_parameters() {
        let script = parse_script("csv_a_tabla.py", CSV).expect("script");
        assert_eq!(script.name, "csv_a_tabla");
        assert_eq!(script.description, "Convierte un CSV en una tabla Markdown.");
        assert_eq!(
            script.parameters,
            vec![
                ScriptParameter { name: "path".into(), kind: ParameterKind::String, required: true, description: "ruta del CSV".into() },
                ScriptParameter { name: "delimiter".into(), kind: ParameterKind::String, required: false, description: "separador; por defecto \",\"".into() },
                ScriptParameter { name: "max_rows".into(), kind: ParameterKind::Integer, required: false, description: String::new() },
            ]
        );
        assert_eq!(script.parameters_schema()["required"], json!(["path"]));
        assert_eq!(script.signature(), "csv_a_tabla(path*: string, delimiter: string, max_rows: integer)");
        // A script of the person without a header still counts.
        let plain = parse_script("limpiar.py", "'''Borra las líneas vacías.\nMás texto.'''\nprint(1)\n").expect("plain");
        assert_eq!((plain.description.as_str(), plain.parameters.len()), ("Borra las líneas vacías.", 0));
        assert_eq!(parse_script("sin_doc.py", "print(1)\n").expect("bare").description, "");
        assert!(parse_script("Mal Nombre.py", CSV).is_none());
        assert!(parse_script("notas.md", CSV).is_none());
    }

    #[test]
    fn arguments_are_checked_against_the_parameters() {
        let script = parse_script("csv_a_tabla.py", CSV).expect("script");
        assert!(script.check_arguments(&json!({ "path": "a.csv", "extra": 1 })).is_ok());
        assert!(script.check_arguments(&json!({ "delimiter": ";" })).expect_err("missing").message.contains("path"));
        assert!(script.check_arguments(&json!({ "path": "a.csv", "max_rows": "diez" })).expect_err("type").message.contains("integer"));
        assert!(script.check_arguments(&json!(["a.csv"])).is_err());
        let run = RunRequest::from_arguments(&json!({ "script": "csv_a_tabla", "arguments": "{\"path\":\"a.csv\"}", "timeoutSeconds": 99999 })).expect("run");
        assert_eq!((run.arguments, run.timeout), (json!({ "path": "a.csv" }), MAX_TIMEOUT));
        assert_eq!(RunRequest::from_arguments(&json!({ "script": "csv_a_tabla" })).expect("run").timeout, DEFAULT_TIMEOUT);
        assert!(RunRequest::from_arguments(&json!({ "script": "../x" })).is_err());
    }

    #[test]
    fn the_run_tool_lists_the_scripts_like_tools() {
        let script = parse_script("csv_a_tabla.py", CSV).expect("script");
        let description = run_tool_description("Ejecuta un script.", &[script.clone()]);
        assert!(description.contains("- csv_a_tabla(path*: string, delimiter: string, max_rows: integer): Convierte un CSV en una tabla Markdown."), "{description}");
        assert!(run_tool_description("Ejecuta.", &[]).contains("Todavía no hay scripts"));
        let many = (0..MAX_DESCRIBED_SCRIPTS + 2).map(|index| AgentScript { name: format!("s{index}"), ..script.clone() }).collect::<Vec<_>>();
        assert!(run_tool_description("x", &many).contains("… y 2 más"));
    }

    #[test]
    fn the_result_reads_json_or_the_end_of_the_output() {
        let json_result = run_result("x", RunEnd::Exited(Some(0)), Duration::from_millis(1234), "{\"filas\": 3}\n", "");
        assert_eq!((json_result["ok"].clone(), json_result["result"]["filas"].clone(), json_result["seconds"].clone()), (json!(true), json!(3), json!(1.2)));
        let text = run_result("x", RunEnd::Exited(Some(2)), Duration::ZERO, &"a".repeat(MAX_OUTPUT_CHARS + 10), "Traceback: boom");
        assert_eq!((text["ok"].clone(), text["exitCode"].clone(), text["stderr"].clone()), (json!(false), json!(2), json!("Traceback: boom")));
        assert!(text["output"].as_str().expect("output").starts_with('…') && text.get("outputCut").is_some() && text.get("note").is_some());
        assert_eq!(run_result("x", RunEnd::TimedOut, Duration::ZERO, "", "")["status"], "se cortó por tiempo");
    }

    #[test]
    fn a_saved_script_gets_the_header_notia_reads() {
        let request = SaveRequest::from_arguments(&json!({
            "name": "csv_a_tabla",
            "description": "Convierte un CSV en una tabla Markdown.",
            "parameters": [{ "name": "path", "type": "string", "required": true, "description": "ruta del CSV" }, { "name": "max_rows", "type": "integer" }],
            "code": "\"\"\"\n---\ndescription: vieja\n---\n\"\"\"\nimport json, sys\nargs = json.load(sys.stdin)\n",
        }))
        .expect("save");
        let source = request.source();
        assert!(source.starts_with("\"\"\"\n---\ndescription: Convierte un CSV en una tabla Markdown.\nparameters:\n  - path (string, requerido): ruta del CSV\n  - max_rows (integer)\n---\n"), "{source}");
        assert!(!source.contains("vieja"), "the code's own header is replaced");
        assert!(source.ends_with("import json, sys\nargs = json.load(sys.stdin)\n"));
        let parsed = parse_script("csv_a_tabla.py", &source).expect("parsed");
        assert_eq!((parsed.description, parsed.parameters), (request.description.clone(), request.parameters.clone()));
        assert_eq!(request.summary(false), "Crear el script de soporte csv_a_tabla");
        assert!(request.detail().starts_with(".agent/scripts/csv_a_tabla.py\n\n\"\"\""));
        assert!(SaveRequest::from_arguments(&json!({ "name": "x", "description": "d", "code": " " })).is_err());
        assert!(SaveRequest::from_arguments(&json!({ "name": "x", "description": "d", "code": "pass", "parameters": [{ "name": "a", "type": "fecha" }] })).is_err());
        assert!(SaveRequest::from_arguments(&json!({ "name": "X-1", "description": "d", "code": "pass" })).is_err());
    }
}
