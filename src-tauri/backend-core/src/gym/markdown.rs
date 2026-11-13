//! Los `.md` del catálogo. Un ejercicio:
//!
//! ```text
//! ---
//! id: bench_press
//! grupo: pecho
//! registro: peso-reps
//! kcalPorMinuto: 6
//! principales: Pecho superior, Pecho inferior
//! secundarios: Tríceps, Deltoide anterior
//! equipamiento: barbell, flat_bench
//! video: Press de banca con barra.mp4
//! alias: Bench Press
//! contexto: "#Personal"
//! ---
//! # Press de banca con barra
//!
//! ![Press de banca con barra](data:image/png;base64,…)
//!
//! ## Cómo se hace
//!
//! 1. Acostate en el banco…
//! ```
//!
//! `video` es opcional: un video de la misma carpeta que se llama como el id
//! (`bench_press.mp4`, `.webm` o `.mov`) es el del ejercicio sin nombrarlo, y
//! al reescribir la ficha no se agrega. Un `video` escrito tiene prioridad.
//!
//! Un equipamiento tiene `id`, `categoria` y `origen` (`catalogo` o
//! `propio`), el título y la foto. Las otras líneas del frontmatter (las que
//! el editor agrega a toda nota) se conservan al reescribir el archivo.

use super::{fold, is_category, is_group, muscle_key, muscle_label, parse_number, Equipment, Exercise, Tracking, MAX_STEPS};

const IMAGE_MARKER: &str = "](data:image/";
const STEPS_HEADING: &str = "como se hace";

const EXERCISE_KEYS: [&str; 10] =
    ["id", "nombre", "grupo", "registro", "kcalporminuto", "principales", "secundarios", "equipamiento", "video", "alias"];
const EQUIPMENT_KEYS: [&str; 4] = ["id", "nombre", "categoria", "origen"];

/// Las líneas `clave: valor` del frontmatter, en orden, y el cuerpo.
fn split_frontmatter(text: &str) -> (Vec<(String, String)>, &str) {
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return (Vec::new(), text);
    };
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), text);
    };
    let fields = rest[..end]
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), unquote(value.trim()).to_string()))
        .filter(|(key, _)| !key.is_empty())
        .collect();
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    (fields, body)
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2 && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\''))) {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

/// Un valor del frontmatter, entre comillas si YAML lo leería de otra forma.
fn front_value(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.contains(['#', ':', '"'])
        || value.starts_with(['[', '{', '&', '*', '!', '|', '>', '%', '@', '`', '\'', '-', '?'])
        || value != value.trim();
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "'"))
    } else {
        value.to_string()
    }
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, value)| value.as_str())
}

fn list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .split(',')
        .map(|item| unquote(item.trim()).trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn extra_front(fields: &[(String, String)], known: &[&str]) -> Vec<(String, String)> {
    fields.iter().filter(|(key, _)| !known.contains(&key.to_ascii_lowercase().as_str())).cloned().collect()
}

/// El título `# …` del cuerpo.
fn heading(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
}

/// La primera imagen embebida: `(alt, data URI)`.
fn embedded_image(body: &str) -> Option<String> {
    let start = body.find(IMAGE_MARKER)?;
    let uri_start = start + 2;
    let end = body[uri_start..].find(')')?;
    Some(body[uri_start..uri_start + end].trim().to_string())
}

/// Los pasos numerados de «## Cómo se hace».
fn steps(body: &str) -> Vec<String> {
    let mut inside = false;
    let mut steps = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("## ") {
            inside = fold(title.trim()) == STEPS_HEADING;
            continue;
        }
        if trimmed.starts_with("# ") {
            inside = false;
            continue;
        }
        if !inside {
            continue;
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            let rest = &trimmed[digits..];
            if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
                if !text.trim().is_empty() && steps.len() < MAX_STEPS {
                    steps.push(text.trim().to_string());
                }
            }
        }
    }
    steps
}

/// Lee un ejercicio; `with_image` también guarda la imagen (para la ficha).
/// `None` si no es un ejercicio (sin id ni título).
pub fn parse_exercise(path: &str, text: &str, with_image: bool) -> Option<Exercise> {
    let (fields, body) = split_frontmatter(text);
    let name = field(&fields, "nombre").map(str::to_string).filter(|name| !name.is_empty()).or_else(|| heading(body))?;
    let id = field(&fields, "id").map(str::trim).filter(|id| !id.is_empty())?.to_string();
    let muscles = |key: &str| {
        let mut keys = Vec::new();
        for item in list(field(&fields, key)) {
            if let Some(muscle) = muscle_key(&item) {
                if !keys.iter().any(|existing| existing == muscle) {
                    keys.push(muscle.to_string());
                }
            }
        }
        keys
    };
    let primary = muscles("principales");
    let secondary = muscles("secundarios").into_iter().filter(|key| !primary.contains(key)).collect();
    let group = field(&fields, "grupo").map(fold).filter(|group| is_group(group)).unwrap_or_else(|| "core".to_string());
    let image = embedded_image(body);
    Some(Exercise {
        id,
        name,
        group,
        tracking: field(&fields, "registro").and_then(Tracking::parse).unwrap_or_default(),
        kcal_per_min: field(&fields, "kcalPorMinuto").and_then(parse_number).filter(|kcal| *kcal >= 0.0).unwrap_or(5.0),
        primary,
        secondary,
        equipment: list(field(&fields, "equipamiento")),
        steps: steps(body),
        video: field(&fields, "video").map(str::trim).filter(|video| !video.is_empty()).map(str::to_string),
        aliases: list(field(&fields, "alias")),
        path: path.to_string(),
        has_image: image.is_some(),
        image: if with_image { image } else { None },
        extra_front: extra_front(&fields, &EXERCISE_KEYS),
    })
}

fn muscle_names(keys: &[String]) -> String {
    keys.iter().filter_map(|key| muscle_label(key)).collect::<Vec<_>>().join(", ")
}

fn front_lines(lines: &[(&str, String)], extra: &[(String, String)]) -> String {
    let mut text = String::from("---\n");
    for (key, value) in lines {
        if !value.is_empty() {
            text.push_str(&format!("{key}: {}\n", front_value(value)));
        }
    }
    for (key, value) in extra {
        text.push_str(&format!("{key}: {}\n", front_value(value)));
    }
    text.push_str("---\n");
    text
}

/// El `.md` de un ejercicio. La imagen sale de `image`: quien reescribe un
/// ejercicio lo lee antes con la imagen.
pub fn render_exercise(exercise: &Exercise) -> String {
    let mut text = front_lines(
        &[
            ("id", exercise.id.clone()),
            ("grupo", exercise.group.clone()),
            ("registro", exercise.tracking.id().to_string()),
            ("kcalPorMinuto", super::fmt(exercise.kcal_per_min, 1).replace(',', ".")),
            ("principales", muscle_names(&exercise.primary)),
            ("secundarios", muscle_names(&exercise.secondary)),
            ("equipamiento", exercise.equipment.join(", ")),
            // El video que se llama como el id se encuentra solo.
            ("video", exercise.video.clone().filter(|video| !super::is_video_of_id(&exercise.id, video)).unwrap_or_default()),
            ("alias", exercise.aliases.join(", ")),
        ],
        &exercise.extra_front,
    );
    text.push_str(&format!("# {}\n", exercise.name.trim()));
    if let Some(image) = &exercise.image {
        text.push_str(&format!("\n![{}]({image})\n", exercise.name.trim().replace(['[', ']'], "")));
    }
    if !exercise.steps.is_empty() {
        text.push_str("\n## Cómo se hace\n\n");
        for (index, step) in exercise.steps.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", index + 1, step.replace('\n', " ").trim()));
        }
    }
    text
}

/// Lee un equipamiento; `None` si no lo es.
pub fn parse_equipment(path: &str, text: &str, with_image: bool) -> Option<Equipment> {
    let (fields, body) = split_frontmatter(text);
    let name = field(&fields, "nombre").map(str::to_string).filter(|name| !name.is_empty()).or_else(|| heading(body))?;
    let id = field(&fields, "id").map(str::trim).filter(|id| !id.is_empty())?.to_string();
    let image = embedded_image(body);
    Some(Equipment {
        id,
        name,
        category: field(&fields, "categoria").map(fold).filter(|category| is_category(category)).unwrap_or_else(|| "otros".to_string()),
        custom: field(&fields, "origen").is_some_and(|origin| fold(origin) == "propio"),
        path: path.to_string(),
        has_image: image.is_some(),
        image: if with_image { image } else { None },
        extra_front: extra_front(&fields, &EQUIPMENT_KEYS),
    })
}

pub fn render_equipment(equipment: &Equipment) -> String {
    let mut text = front_lines(
        &[
            ("id", equipment.id.clone()),
            ("categoria", equipment.category.clone()),
            ("origen", if equipment.custom { "propio" } else { "catalogo" }.to_string()),
        ],
        &equipment.extra_front,
    );
    text.push_str(&format!("# {}\n", equipment.name.trim()));
    if let Some(image) = &equipment.image {
        text.push_str(&format!("\n![{}]({image})\n", equipment.name.trim().replace(['[', ']'], "")));
    }
    text
}
