//! Images the agent generates or edits with Qwen-Image 2.1 Turbo (Q4)
//! through stable-diffusion.cpp's `sd-cli`: the model files, the tool
//! arguments, the image size, the command line and the result the model
//! reads. The adapter downloads the files, runs `sd-cli` and keeps the image
//! with the reply (the chat file, or a Telegram photo).

use serde_json::{json, Value};

use crate::error::{BackendError, BackendErrorCode};

pub const GENERATE_IMAGE_TOOL: &str = "generate_image";
pub const EDIT_IMAGE_TOOL: &str = "edit_image";
pub const IMAGE_TOOLS: [&str; 2] = [GENERATE_IMAGE_TOOL, EDIT_IMAGE_TOOL];

pub fn is_image_tool(name: &str) -> bool {
    IMAGE_TOOLS.contains(&name)
}

/// Folder of the model files under the app data of the device.
pub const MODEL_PROFILE: &str = "qwen-image-2.1-turbo-q4";

/// What a model file is for in the `sd-cli` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    /// The 7B image model (denoiser).
    Diffusion,
    /// Qwen3-VL-8B, which reads the prompt.
    TextEncoder,
    /// The vision projector of the encoder, which reads the image to edit.
    Vision,
    Vae,
}

/// A file of the model, pinned by size and SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub role: ModelRole,
}

/// Qwen-Image 2.1 Turbo AD-Q4_K, its encoder Qwen3-VL-8B AD-Q4_K (the
/// abliterated one, which draws the same images), the encoder's vision
/// projector and the Qwen-Image 2.1 VAE.
pub const MODEL_FILES: [ModelFile; 4] = [
    ModelFile {
        name: "Qwen-Image-2.1-Turbo-AD-Q4_K.gguf",
        url: "https://huggingface.co/AtomicChat/Qwen-Image-2.1-Turbo-GGUF/resolve/main/Qwen-Image-2.1-Turbo-AD-Q4_K.gguf",
        size: 4_201_694_944,
        sha256: "4bb73c53cbe284bbd6d69b9fdc59539c531f0389dd2aa567663d777f8130bc59",
        role: ModelRole::Diffusion,
    },
    ModelFile {
        name: "Qwen-Image-2.1-Turbo-Abliterated-Uncensored-AD-Q4_K.gguf",
        url: "https://huggingface.co/AtomicChat/Qwen-Image-2.1-Turbo-Uncensored-GGUF/resolve/main/Qwen-Image-2.1-Turbo-Abliterated-Uncensored-AD-Q4_K.gguf",
        size: 5_287_517_120,
        sha256: "8123f36d2b3e562deef164dffdae33bdb1d152797ad10bb630827eaa7bce795d",
        role: ModelRole::TextEncoder,
    },
    ModelFile {
        name: "mmproj-Qwen-Image-2.1-Turbo-Abliterated-Uncensored-F16.gguf",
        url: "https://huggingface.co/AtomicChat/Qwen-Image-2.1-Turbo-Uncensored-GGUF/resolve/main/mmproj-Qwen-Image-2.1-Turbo-Abliterated-Uncensored-F16.gguf",
        size: 1_159_029_984,
        sha256: "d9870a1f168db3d814596216aaf35702154147b443572c7939acca1802621ac6",
        role: ModelRole::Vision,
    },
    ModelFile {
        name: "qwen_image_2.1_vae_bf16.safetensors",
        url: "https://huggingface.co/Comfy-Org/Qwen-Image-2.1/resolve/main/vae/qwen_image_2.1_vae_bf16.safetensors",
        size: 675_509_688,
        sha256: "bb21f7473051e1ac368515dd3f2e15cd44d7a11748ee8823e1ddca3e4876b7c9",
        role: ModelRole::Vae,
    },
];

/// Bytes of every model file together.
pub fn total_model_bytes() -> u64 {
    MODEL_FILES.iter().map(|file| file.size).sum()
}

/// The file of `role`.
pub fn model_file(role: ModelRole) -> &'static ModelFile {
    MODEL_FILES.iter().find(|file| file.role == role).expect("every role has a file")
}

/// The fixed 8-step schedule the Turbo model was distilled for; without it
/// the default schedule draws different images.
pub const TURBO_SIGMAS: &str = "1.0,0.978453,0.95418,0.926626,0.89508,0.845148,0.704534,0.414568,0.0";
pub const TURBO_STEPS: u32 = 8;
const MAX_PROMPT_CHARS: usize = 4_000;
/// Sides are multiples of this, as the model needs.
const SIDE_STEP: u32 = 32;
const MIN_SIDE: u32 = 256;
const MAX_SIDE: u32 = 2_048;
/// About two megapixels, the most Qwen recommends.
const MAX_PIXELS: u64 = 2_097_152;
const DEFAULT_SIDE: u32 = 1_024;
/// Longest reference name an edit accepts.
const MAX_REFERENCE_CHARS: usize = 255;

/// Width and height of each aspect the tools accept, about one megapixel.
const ASPECTS: [(&str, u32, u32); 7] = [
    ("1:1", 1024, 1024),
    ("16:9", 1344, 768),
    ("9:16", 768, 1344),
    ("4:3", 1184, 896),
    ("3:4", 896, 1184),
    ("3:2", 1248, 832),
    ("2:3", 832, 1248),
];

/// One run of `sd-cli`: what to draw (or how to change the image
/// `reference`), its size and its seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageJob {
    pub prompt: String,
    /// `None` until an edit takes the size of its image.
    pub size: Option<(u32, u32)>,
    pub seed: i64,
    /// The image an edit starts from: its reference in the conversation.
    pub reference: Option<String>,
}

fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::invalid_input(message)
}

fn text(arguments: &Value, name: &str) -> Option<String> {
    arguments.get(name).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn whole(arguments: &Value, name: &str) -> Result<Option<i64>, BackendError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .or_else(|| value.as_f64().filter(|number| number.fract() == 0.0).map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.trim().parse::<i64>().ok()))
            .map(Some)
            .ok_or_else(|| invalid(format!("{name} tiene que ser un número entero."))),
    }
}

/// `width` × `height` brought within the model's limits keeping the
/// aspect: at most `MAX_SIDE` per side and `MAX_PIXELS` in total, at least
/// `MIN_SIDE`, and both sides multiples of 32.
pub fn fit_size(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let mut scale = (MAX_PIXELS as f64 / (width * height)).sqrt().min(1.0);
    scale = scale.min(f64::from(MAX_SIDE) / width.max(height));
    scale = scale.max(f64::from(MIN_SIDE) / width.min(height));
    let snap = |side: f64| {
        let steps = (side / f64::from(SIDE_STEP)).floor().max(1.0) as u32;
        (steps * SIDE_STEP).clamp(MIN_SIDE, MAX_SIDE)
    };
    (snap(width * scale), snap(height * scale))
}

/// The size a call asks for: `aspect` (`16:9`…), or `width` and `height`
/// (one alone keeps the image square), fitted with `fit_size`; `None` when
/// it asks for none.
fn requested_size(arguments: &Value) -> Result<Option<(u32, u32)>, BackendError> {
    if let Some(aspect) = text(arguments, "aspect") {
        let compact = aspect.replace(' ', "").replace('x', ":");
        return ASPECTS
            .iter()
            .find(|(name, _, _)| *name == compact)
            .map(|(_, width, height)| Some((*width, *height)))
            .ok_or_else(|| invalid(format!("aspect tiene que ser uno de {}.", ASPECTS.map(|(name, _, _)| name).join(", "))));
    }
    let side = |name| -> Result<Option<u32>, BackendError> {
        match whole(arguments, name)? {
            None => Ok(None),
            Some(value) if value > 0 && value <= 8_192 => Ok(Some(value as u32)),
            Some(_) => Err(invalid(format!("{name} tiene que estar entre 1 y 8192 píxeles."))),
        }
    };
    Ok(match (side("width")?, side("height")?) {
        (None, None) => None,
        (Some(width), Some(height)) => Some(fit_size(width, height)),
        (Some(side), None) | (None, Some(side)) => Some(fit_size(side, side)),
    })
}

fn prompt(arguments: &Value, name: &str) -> Result<String, BackendError> {
    let prompt = text(arguments, name).ok_or_else(|| invalid(format!("Falta {name}.")))?;
    let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(invalid(format!("{name} puede tener hasta {MAX_PROMPT_CHARS} caracteres.")));
    }
    if sexualizes_minors(&prompt) {
        return Err(BackendError::new(BackendErrorCode::Forbidden, MINORS_REFUSAL, false));
    }
    Ok(prompt)
}

/// The only content the image tools refuse, whatever the path (`/image`,
/// `generate_image` or `edit_image`).
pub const MINORS_REFUSAL: &str = "No se generan imágenes sexuales ni de desnudos con menores de edad.";

/// Words for a minor, folded. A trailing `*` takes any ending.
const MINOR_TERMS: &[&str] = &[
    "nino", "nina", "ninos", "ninas", "ninit*", "nene", "nena", "nenes", "nenas", "nenit*", "menor", "menores",
    "infantil*", "infante", "infantes", "infancia", "adolescent*", "preadolescent*", "puber*", "prepuber*",
    "colegial*", "bebe", "bebes", "bebit*", "chiquill*", "ninfet*", "child*", "kid", "kids", "kiddie*", "teen",
    "teens", "teenage*", "preteen*", "tween", "tweens", "underage*", "minor", "minors", "toddler*", "infant",
    "infants", "pubescent*", "prepubescent*", "schoolgirl*", "schoolboy*", "loli", "lolis", "lolicon", "lolita*",
    "shota", "shotas", "shotacon", "jailbait",
];

/// Words of nudity or sex, folded. A trailing `*` takes any ending; where a
/// stem would also start another word (`sex` → `sexto`, `nud` → `nudo`) the
/// words are listed whole.
const SEXUAL_TERMS: &[&str] = &[
    "desnud*", "desvest*", "sin ropa", "ropa interior", "sexual*", "sexo", "sexy", "sexi", "sex", "porn*", "erot*",
    "nsfw", "hentai", "ecchi", "lewd", "xxx", "nude", "nudes", "nudity", "nudist*", "nudism*", "naked", "topless",
    "lencer*", "lingerie", "underwear", "panties", "bombacha*", "calzon*", "tanga", "tangas", "genital*", "sensual*",
    "seduc*", "provocativ*", "sugerente*", "suggestive", "orgasm*", "masturb*", "coito", "pene", "penis", "vagina*",
    "tetas", "boobs", "tits", "pezon*", "nipple*",
];

/// Ages under 18 written out (`doce años`). `un`/`one` stay out: «hace un
/// año» says when, not how old.
const SPELLED_MINOR_AGES: &[&str] = &[
    "dos", "tres", "cuatro", "cinco", "seis", "siete", "ocho", "nueve", "diez", "once", "doce", "trece", "catorce",
    "quince", "dieciseis", "diecisiete", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen",
];

/// What follows a number to make it an age: `15 años`, `12 yo`,
/// `14-year-old` (`y/o` is read apart, as `y o`).
const AGE_UNITS: &[&str] = &["anos", "ano", "anitos", "anito", "years", "year", "yrs", "yr", "yo"];

const ADULT_AGE: u32 = 18;

/// Characters that do not show and would split a word in two
/// (`des\u{200B}nuda`).
const INVISIBLE: [char; 6] = ['\u{00AD}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'];

/// `text` lowercase, without accents (also written as combining marks), `ñ`
/// as `n` and without invisible characters, so `NIÑA`, `niña` and `nina`
/// read the same.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|character| !INVISIBLE.contains(character) && !('\u{0300}'..='\u{036F}').contains(character))
        .map(|character| match character {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// The folded words of `text`, one space apart and with a space at each
/// end, a number apart from the letters next to it: `«Niña» de 15años` →
/// ` nina de 15 anos `.
fn folded_words(text: &str) -> String {
    let mut line = String::from(" ");
    for character in fold(text).chars() {
        let last = line.chars().next_back().unwrap_or(' ');
        if !character.is_alphanumeric() {
            if last != ' ' {
                line.push(' ');
            }
            continue;
        }
        if last != ' ' && last.is_ascii_digit() != character.is_ascii_digit() {
            line.push(' ');
        }
        line.push(character);
    }
    if !line.ends_with(' ') {
        line.push(' ');
    }
    line
}

/// Whether `line` (see `folded_words`) has one of `terms`.
fn has_any(line: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| match term.strip_suffix('*') {
        Some(stem) => line.contains(&format!(" {stem}")),
        None => line.contains(&format!(" {term} ")),
    })
}

/// Whether `line` (see `folded_words`) gives an age under 18.
fn names_a_minor_age(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    words.iter().enumerate().any(|(index, word)| {
        let minor = word.parse::<u32>().is_ok_and(|age| age < ADULT_AGE) || SPELLED_MINOR_AGES.contains(word);
        let unit = match (words.get(index + 1), words.get(index + 2)) {
            (Some(&"y"), Some(&"o")) => true,
            (Some(next), _) => AGE_UNITS.contains(next),
            (None, _) => false,
        };
        minor && unit
    })
}

/// Whether `prompt` asks for nudity or sex with a minor: a word for a minor,
/// or an age under 18, together with a word of nudity or sex anywhere in the
/// same prompt. Adult content and any other subject pass.
fn sexualizes_minors(prompt: &str) -> bool {
    let line = folded_words(prompt);
    (has_any(&line, MINOR_TERMS) || names_a_minor_age(&line)) && has_any(&line, SEXUAL_TERMS)
}

/// A `/image` message (also `/imagen`): what the person wants drawn, or how
/// to change the image they attached, sent to the model as they wrote it,
/// without the chat model in between. An aspect (`16:9`…) may go first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageCommand {
    pub prompt: String,
    pub aspect: Option<&'static str>,
}

/// How to use `/image`.
pub const IMAGE_COMMAND_HELP: &str =
    "Escribí qué imagen querés después del comando, por ejemplo: /image un faro al atardecer. Podés empezar con una proporción (/image 16:9 …) o adjuntar una imagen para editarla.";

/// The `/image` command of a message: `None` when the message is not one,
/// an error when it says nothing to draw.
pub fn parse_image_command(text: &str) -> Option<Result<ImageCommand, BackendError>> {
    let text = text.trim_start();
    let (command, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if !matches!(command.to_lowercase().as_str(), "/image" | "/imagen") {
        return None;
    }
    let rest = rest.trim();
    let (first, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let (aspect, prompt) = match ASPECTS.iter().find(|(name, _, _)| *name == first) {
        Some((name, _, _)) => (Some(*name), after.trim()),
        None => (None, rest),
    };
    if prompt.is_empty() {
        return Some(Err(invalid(IMAGE_COMMAND_HELP)));
    }
    Some(Ok(ImageCommand { prompt: prompt.to_string(), aspect }))
}

/// The tool arguments of a command: an edit of `reference` (the attached
/// image) or a new image.
pub fn command_arguments(command: &ImageCommand, reference: Option<&str>) -> Value {
    let mut arguments = match reference {
        Some(reference) => json!({ "image": reference, "instruction": command.prompt }),
        None => json!({ "prompt": command.prompt }),
    };
    if let Some(aspect) = command.aspect {
        arguments["aspect"] = Value::String(aspect.to_string());
    }
    arguments
}

/// What the reply to a command says next to the image.
pub fn command_reply(result: &Value) -> String {
    let (width, height, seed) = (result["width"].as_u64().unwrap_or(0), result["height"].as_u64().unwrap_or(0), result["seed"].as_i64().unwrap_or(0));
    match result["editedFrom"].as_str() {
        Some(source) => format!("Listo: edité «{source}» ({width} × {height}, semilla {seed})."),
        None => format!("Listo: imagen de {width} × {height} (semilla {seed})."),
    }
}

/// The seed a call names, or `random_seed` (non-negative).
fn seed(arguments: &Value, random_seed: i64) -> Result<i64, BackendError> {
    Ok(whole(arguments, "seed")?.filter(|seed| *seed >= 0).unwrap_or(random_seed.abs()))
}

/// The run of a `generate_image` call: `prompt`, optional `aspect` or
/// `width`/`height` (1024 × 1024 by default) and optional `seed`.
pub fn generate_job(arguments: &Value, random_seed: i64) -> Result<ImageJob, BackendError> {
    Ok(ImageJob {
        prompt: prompt(arguments, "prompt")?,
        size: Some(requested_size(arguments)?.unwrap_or((DEFAULT_SIDE, DEFAULT_SIDE))),
        seed: seed(arguments, random_seed)?,
        reference: None,
    })
}

/// The run of an `edit_image` call: `image` (the reference of an image of
/// the conversation), `instruction`, and optional size and seed. Without a
/// size it keeps the image's (the adapter fills it in).
pub fn edit_job(arguments: &Value, random_seed: i64) -> Result<ImageJob, BackendError> {
    let reference = text(arguments, "image").ok_or_else(|| invalid("Falta image: la referencia de la imagen a editar."))?;
    if reference.chars().count() > MAX_REFERENCE_CHARS || reference.contains(['\n', '\r']) {
        return Err(invalid("La referencia de la imagen no es válida."));
    }
    Ok(ImageJob {
        prompt: prompt(arguments, "instruction")?,
        size: requested_size(arguments)?,
        seed: seed(arguments, random_seed)?,
        reference: Some(reference),
    })
}

/// Paths of the model files on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    pub diffusion: String,
    pub text_encoder: String,
    pub vision: String,
    pub vae: String,
}

/// Arguments of `sd-cli` for `job`, of size `width` × `height`, drawing to
/// `output`. An edit passes its image (`reference_path`) and the vision
/// projector. The weights stay in RAM and move to the GPU when each stage
/// needs them, so the model and its encoder fit a 12 GB card together, and
/// the VAE decodes in tiles (at full size it does not fit next to them).
pub fn sd_cli_arguments(
    job: &ImageJob,
    (width, height): (u32, u32),
    models: &ModelPaths,
    reference_path: Option<&str>,
    output: &str,
) -> Vec<String> {
    let mut arguments = vec![
        "--diffusion-model".to_string(),
        models.diffusion.clone(),
        "--vae".to_string(),
        models.vae.clone(),
        "--llm".to_string(),
        models.text_encoder.clone(),
    ];
    if let Some(reference) = reference_path {
        arguments.extend(["--llm_vision".to_string(), models.vision.clone(), "-r".to_string(), reference.to_string()]);
    }
    arguments.extend(
        [
            "-p",
            &job.prompt,
            "--steps",
            &TURBO_STEPS.to_string(),
            "--sigmas",
            TURBO_SIGMAS,
            "--cfg-scale",
            "1.0",
            "--sampling-method",
            "euler",
            "-W",
            &width.to_string(),
            "-H",
            &height.to_string(),
            "-s",
            &job.seed.to_string(),
            "--offload-to-cpu",
            "--diffusion-fa",
            "--vae-tiling",
            "-o",
            output,
        ]
        .map(str::to_string),
    );
    arguments
}

/// File name of a new image, from the local time `stamp`
/// (`AAAA-MM-DD-HH-MM-SS`) and its number in the turn, so the images of a
/// chat have different references.
pub fn image_file_name(stamp: &str, number: usize) -> String {
    let stamp = stamp.chars().filter(|character| character.is_ascii_digit() || *character == '-').collect::<String>();
    if number <= 1 { format!("imagen-{stamp}.png") } else { format!("imagen-{stamp}-{number}.png") }
}

/// What the model reads after an image was made: its reference, size and
/// seed, and that the image already goes with the reply.
pub fn tool_result(name: &str, (width, height): (u32, u32), seed: i64, edited_from: Option<&str>) -> Value {
    let mut result = json!({
        "image": name,
        "width": width,
        "height": height,
        "seed": seed,
        "delivered": "La imagen ya va adjunta a tu respuesta: no la describas como pendiente ni pegues un enlace. Para cambiarla, usá edit_image con esta referencia.",
    });
    if let Some(source) = edited_from {
        result["editedFrom"] = Value::String(source.to_string());
    }
    result
}

/// What `sd-cli` printed, as an error the person understands. The output
/// never reaches the log or the model as is: it may repeat the prompt.
pub fn run_error(output: &str) -> BackendError {
    let lowered = output.to_ascii_lowercase();
    let message = if lowered.contains("out of memory") || lowered.contains("outofmemory") || lowered.contains("failed to allocate") {
        "La placa de video se quedó sin memoria generando la imagen. Cerrá lo que use la GPU (por ejemplo Meeting) o pedí una imagen más chica."
    } else if lowered.contains("vulkan") && lowered.contains("not") && lowered.contains("found") {
        "No se encontró un driver Vulkan para usar la placa de video."
    } else {
        "No se pudo generar la imagen con el modelo local."
    };
    BackendError::new(crate::error::BackendErrorCode::Internal, message, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_files_are_pinned() {
        assert_eq!(MODEL_FILES.len(), 4);
        assert_eq!(total_model_bytes(), 11_323_751_736);
        for file in MODEL_FILES {
            assert!(file.url.starts_with("https://huggingface.co/") && file.url.ends_with(file.name));
            assert_eq!(file.sha256.len(), 64);
        }
        assert_eq!(model_file(ModelRole::Vision).name, "mmproj-Qwen-Image-2.1-Turbo-Abliterated-Uncensored-F16.gguf");
    }

    #[test]
    fn sizes_keep_the_aspect_within_the_limits() {
        assert_eq!(fit_size(1024, 1024), (1024, 1024));
        assert_eq!(fit_size(4000, 3000), (1664, 1248));
        assert_eq!(fit_size(1000, 1000), (992, 992));
        assert_eq!(fit_size(100, 50), (512, 256));
        assert_eq!(fit_size(8192, 256), (2048, 256));
        let (width, height) = fit_size(3000, 3000);
        assert!(u64::from(width) * u64::from(height) <= MAX_PIXELS && width % 32 == 0);
    }

    #[test]
    fn generate_calls_become_jobs() {
        let job = generate_job(&json!({ "prompt": "  un gato   con anteojos " }), -7).expect("job");
        assert_eq!(job, ImageJob { prompt: "un gato con anteojos".into(), size: Some((1024, 1024)), seed: 7, reference: None });
        let wide = generate_job(&json!({ "prompt": "paisaje", "aspect": "16 : 9", "seed": "42" }), 1).expect("job");
        assert_eq!((wide.size, wide.seed), (Some((1344, 768)), 42));
        let sized = generate_job(&json!({ "prompt": "x", "width": 1920, "height": 1080 }), 1).expect("job");
        assert_eq!(sized.size, Some((1920, 1056)));
        assert!(generate_job(&json!({ "prompt": " " }), 1).is_err());
        assert!(generate_job(&json!({ "prompt": "x", "aspect": "5:4" }), 1).is_err());
        assert!(generate_job(&json!({ "prompt": "x", "width": 0 }), 1).is_err());
        assert!(generate_job(&json!({ "prompt": "x".repeat(MAX_PROMPT_CHARS + 1) }), 1).is_err());
    }

    #[test]
    fn edit_calls_name_their_image_and_may_keep_its_size() {
        let job = edit_job(&json!({ "image": "imagen-2026-10-13.png", "instruction": "poné el cielo de noche" }), 3).expect("job");
        assert_eq!(job.reference.as_deref(), Some("imagen-2026-10-13.png"));
        assert_eq!((job.size, job.prompt.as_str()), (None, "poné el cielo de noche"));
        assert!(edit_job(&json!({ "instruction": "x" }), 1).is_err());
        assert!(edit_job(&json!({ "image": "a.png" }), 1).is_err());
    }

    #[test]
    fn image_commands_go_to_the_model_as_written() {
        assert_eq!(parse_image_command("hola"), None);
        assert_eq!(parse_image_command("/images de ayer"), None);
        let command = parse_image_command("  /IMAGE   una mujer desnuda en la playa, óleo ").expect("command").expect("valid");
        assert_eq!(command, ImageCommand { prompt: "una mujer desnuda en la playa, óleo".into(), aspect: None });
        let wide = parse_image_command("/imagen 16:9 un bosque").expect("command").expect("valid");
        assert_eq!((wide.prompt.as_str(), wide.aspect), ("un bosque", Some("16:9")));
        assert_eq!(parse_image_command("/image").expect("command").expect_err("empty").message, IMAGE_COMMAND_HELP);
        assert!(parse_image_command("/image 9:16").expect("command").is_err());

        let new = command_arguments(&wide, None);
        assert_eq!((new["prompt"].as_str(), new["aspect"].as_str()), (Some("un bosque"), Some("16:9")));
        let job = generate_job(&new, 1).expect("job");
        assert_eq!(job.size, Some((1344, 768)));
        let edit = command_arguments(&command, Some("foto.jpg"));
        assert_eq!((edit["image"].as_str(), edit["instruction"].as_str()), (Some("foto.jpg"), Some(command.prompt.as_str())));
        assert!(edit_job(&edit, 1).is_ok());

        assert_eq!(command_reply(&tool_result("imagen-1.png", (1024, 1024), 7, None)), "Listo: imagen de 1024 × 1024 (semilla 7).");
        assert_eq!(command_reply(&tool_result("imagen-2.png", (512, 512), 8, Some("foto.jpg"))), "Listo: edité «foto.jpg» (512 × 512, semilla 8).");
    }

    #[test]
    fn only_sexual_content_with_minors_is_refused() {
        for allowed in [
            "una mujer desnuda en la playa",
            "pareja adulta, escena erótica, nsfw",
            "un niño jugando a la pelota en el parque",
            "retrato de una modelo de 25 años en lencería",
            "logo de una tienda infantil",
        ] {
            assert!(generate_job(&json!({ "prompt": allowed }), 1).is_ok(), "{allowed}");
        }
        for refused in ["niña desnuda", "chica de 15 años en topless", "teen nude photo", "Adolescente, escena sexual", "loli hentai"] {
            let error = generate_job(&json!({ "prompt": refused }), 1).expect_err(refused);
            assert_eq!(error.code, crate::error::BackendErrorCode::Forbidden, "{refused}");
        }
        assert!(edit_job(&json!({ "image": "a.png", "instruction": "dejala desnuda, que parezca de 12 años" }), 1).is_err());
    }

    #[test]
    fn the_refusal_reads_case_accents_and_how_ages_are_written() {
        for refused in [
            "NIÑA DESNUDA",
            "nina desnuda",
            "nin\u{0303}a desnuda",
            "una niña des\u{200B}nuda",
            "Niñito sin ropa",
            "chica de 15años, topless",
            "girl, 14-year-old, naked",
            "chica 16 y/o en lencería",
            "adolescente de doce años, erótica",
            "Bebé desnudo",
            "menores de edad, escena de SEXO",
        ] {
            let error = generate_job(&json!({ "prompt": refused }), 1).expect_err(refused);
            assert_eq!((error.code, error.message.as_str()), (BackendErrorCode::Forbidden, MINORS_REFUSAL), "{refused}");
        }
        for allowed in [
            "mujer de 30 años desnuda",
            "pareja desnuda, los dos de 18 años",
            "una niña de sexto grado en el acto escolar",
            "un niño haciendo un nudo marinero",
            "sexteto de jazz tocando para chicos",
            "una pareja desnuda con 2 gatos",
        ] {
            assert!(generate_job(&json!({ "prompt": allowed }), 1).is_ok(), "{allowed}");
        }
    }

    #[test]
    fn the_command_follows_the_turbo_settings() {
        let models = ModelPaths { diffusion: "d.gguf".into(), text_encoder: "t.gguf".into(), vision: "v.gguf".into(), vae: "vae.st".into() };
        let job = ImageJob { prompt: "un faro".into(), size: Some((1024, 768)), seed: 5, reference: None };
        let arguments = sd_cli_arguments(&job, (1024, 768), &models, None, "out.png");
        let joined = arguments.join(" ");
        assert!(joined.starts_with("--diffusion-model d.gguf --vae vae.st --llm t.gguf -p un faro --steps 8"));
        assert!(joined.contains(&format!("--sigmas {TURBO_SIGMAS} --cfg-scale 1.0 --sampling-method euler -W 1024 -H 768 -s 5")));
        assert!(joined.ends_with("--offload-to-cpu --diffusion-fa --vae-tiling -o out.png") && !joined.contains("--llm_vision"));
        let edit = sd_cli_arguments(&job, (1024, 768), &models, Some("ref.png"), "out.png").join(" ");
        assert!(edit.contains("--llm t.gguf --llm_vision v.gguf -r ref.png -p un faro"));
    }

    #[test]
    fn names_results_and_errors() {
        assert_eq!(image_file_name("2026-10-13-01-30-00", 1), "imagen-2026-10-13-01-30-00.png");
        assert_eq!(image_file_name("2026-10-13 01:30", 2), "imagen-2026-10-130130-2.png");
        let result = tool_result("imagen-1.png", (1024, 768), 9, Some("telegram-x.jpg"));
        assert_eq!(result["image"], "imagen-1.png");
        assert_eq!((result["width"].as_u64(), result["seed"].as_i64()), (Some(1024), Some(9)));
        assert_eq!(result["editedFrom"], "telegram-x.jpg");
        assert!(run_error("ggml_vulkan: Device memory allocation failed: ErrorOutOfDeviceMemory out of memory").message.contains("sin memoria"));
        assert_eq!(run_error("algo raro").message, "No se pudo generar la imagen con el modelo local.");
    }
}
