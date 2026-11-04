use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// Set while this device does not run the library's services
/// (`device_role`): another computer answers Telegram, so no call leaves
/// this device, whatever asks for it.
static CLIENT_DEVICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Blocks (or allows again) every call to Telegram from this device.
pub(crate) fn set_client_device(client: bool) {
    CLIENT_DEVICE.store(client, std::sync::atomic::Ordering::SeqCst);
}

/// The Telegram API, unless another computer answers Telegram.
fn api_base() -> Result<&'static str, String> {
    if CLIENT_DEVICE.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Este dispositivo no atiende Telegram: lo atiende la computadora de la biblioteca con más procesamiento.".to_string());
    }
    Ok(TELEGRAM_API_BASE)
}
const MAX_MESSAGE_CHARS: usize = 4_000;
const MAX_AUDIO_BYTES: u64 = 20 * 1024 * 1024;
const MAX_AUDIO_DURATION_SECONDS: u32 = 15 * 60;
const MAX_PHOTO_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PHOTO_PIXELS: u64 = 16_000_000;
const MAX_DOCUMENT_BYTES: u64 = 15 * 1024 * 1024;
/// An image sent as a file keeps its full resolution; screenshots and
/// uncompressed photos fit in this.
const MAX_IMAGE_DOCUMENT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramSentMessage {
    #[serde(rename = "message_id")]
    message_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramIdentity {
    pub id: i64,
    pub username: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    username: Option<String>,
    first_name: String,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    text: Option<String>,
    caption: Option<String>,
    photo: Option<Vec<TelegramPhoto>>,
    document: Option<TelegramDocument>,
    voice: Option<TelegramAudio>,
    audio: Option<TelegramAudio>,
    /// Shared by the photos or files sent together as an album.
    media_group_id: Option<String>,
    /// The message this one answers (Telegram's «Responder»).
    reply_to_message: Option<TelegramReplyRef>,
}

#[derive(Debug, Deserialize)]
struct TelegramReplyRef {
    message_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramAudio {
    #[serde(alias = "file_id")]
    file_id: String,
    duration: u32,
    #[serde(alias = "mime_type")]
    mime_type: Option<String>,
    #[serde(alias = "file_size")]
    file_size: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TelegramPhoto {
    #[serde(alias = "file_id")]
    pub file_id: String,
    #[serde(alias = "file_size")]
    pub file_size: Option<u64>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TelegramDocument {
    #[serde(alias = "file_id")]
    pub file_id: String,
    #[serde(alias = "file_name")]
    pub file_name: Option<String>,
    #[serde(alias = "mime_type")]
    pub mime_type: Option<String>,
    #[serde(alias = "file_size")]
    pub file_size: Option<u64>,
}

/// What a document sent to the bot is, for the request that receives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Pdf,
    /// A photo or screenshot sent as a file (JPEG or PNG).
    Image,
}

/// Kind of a document, or `None` when the bot does not read it.
pub fn document_kind(document: &TelegramDocument) -> Option<DocumentKind> {
    let name = document.file_name.as_deref().unwrap_or_default().to_ascii_lowercase();
    let media_type = document.mime_type.as_deref().unwrap_or_default().to_ascii_lowercase();
    if media_type == "application/pdf" || name.ends_with(".pdf") {
        return Some(DocumentKind::Pdf);
    }
    let image_name = [".jpg", ".jpeg", ".png"].iter().any(|extension| name.ends_with(extension));
    (matches!(media_type.as_str(), "image/jpeg" | "image/png") || (media_type.is_empty() && image_name))
        .then_some(DocumentKind::Image)
}

#[derive(Debug, Deserialize)]
struct TelegramFile {
    file_path: Option<String>,
    file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackQuery {
    id: String,
    from: TelegramUser,
    message: Option<TelegramMessage>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    callback_query: Option<TelegramCallbackQuery>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingTelegramUpdate {
    pub update_id: i64,
    pub chat_id: i64,
    pub user: TelegramIdentity,
    pub message_id: Option<i64>,
    pub text: Option<String>,
    pub audio: Option<TelegramAudio>,
    pub photo: Option<TelegramPhoto>,
    pub document: Option<TelegramDocument>,
    /// Album the photo or file belongs to; its parts arrive as separate updates.
    pub media_group_id: Option<String>,
    /// The bot message this one answers, when the person used «Responder».
    pub reply_to_message_id: Option<i64>,
    pub callback_query_id: Option<String>,
    pub callback_data: Option<String>,
    pub chat_type: String,
}

fn endpoint(token: &str, method: &str) -> Result<String, String> {
    let token = token.trim();
    if token.is_empty()
        || token.len() > 256
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte))
    {
        return Err("El token de Telegram tiene un formato invalido.".to_string());
    }
    Ok(format!("{}/bot{token}/{method}", api_base()?))
}

fn identity(user: TelegramUser) -> TelegramIdentity {
    let display_name = match user.last_name.as_deref() {
        Some(last_name) if !last_name.trim().is_empty() => {
            format!("{} {last_name}", user.first_name)
        }
        _ => user.first_name.clone(),
    };
    TelegramIdentity {
        id: user.id,
        username: user.username,
        display_name,
    }
}

async fn decode<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T, String> {
    let payload = response
        .json::<TelegramResponse<T>>()
        .await
        .map_err(|_| "Telegram devolvio una respuesta invalida.".to_string())?;
    if !payload.ok {
        return Err(payload
            .description
            .unwrap_or_else(|| "Telegram rechazo la solicitud.".to_string()));
    }
    payload
        .result
        .ok_or_else(|| "Telegram no devolvio un resultado.".to_string())
}

pub async fn check_bot(token: &str) -> Result<TelegramIdentity, String> {
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .get(endpoint(token, "getMe")?)
        .send()
        .await
        .map_err(|_| "No se pudo conectar con Telegram.".to_string())?;
    decode::<TelegramUser>(response).await.map(identity)
}

pub async fn get_updates(token: &str, offset: i64) -> Result<Vec<IncomingTelegramUpdate>, String> {
    let response = Client::builder().timeout(Duration::from_secs(35)).build().map_err(|error| error.to_string())?
        .post(endpoint(token, "getUpdates")?)
        .json(&serde_json::json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message", "callback_query"] }))
        .send().await.map_err(|_| "Se interrumpio la conexion con Telegram.".to_string())?;
    let updates = decode::<Vec<TelegramUpdate>>(response).await?;
    Ok(updates
        .into_iter()
        .filter_map(|update| {
            if let Some(message) = update.message {
                let user = message.from.map(identity)?;
                return Some(IncomingTelegramUpdate {
                    update_id: update.update_id,
                    chat_id: message.chat.id,
                    user,
                    message_id: Some(message.message_id),
                    text: message.text.or(message.caption),
                    audio: message.voice.or(message.audio),
                    photo: message.photo.and_then(|photos| {
                        photos
                            .into_iter()
                            .max_by_key(|photo| u64::from(photo.width) * u64::from(photo.height))
                    }),
                    document: message.document,
                    media_group_id: message.media_group_id,
                    reply_to_message_id: message.reply_to_message.map(|reply| reply.message_id),
                    callback_query_id: None,
                    callback_data: None,
                    chat_type: message.chat.chat_type,
                });
            }
            let callback = update.callback_query?;
            let callback_message = callback.message?;
            let chat_id = callback_message.chat.id;
            Some(IncomingTelegramUpdate {
                update_id: update.update_id,
                chat_id,
                user: identity(callback.from),
                message_id: None,
                text: None,
                audio: None,
                photo: None,
                document: None,
                media_group_id: None,
                reply_to_message_id: None,
                callback_query_id: Some(callback.id),
                callback_data: callback.data,
                chat_type: callback_message.chat.chat_type,
            })
        })
        .collect())
}

pub async fn send_message(
    token: &str,
    chat_id: i64,
    text: &str,
    buttons: Vec<(String, String)>,
    parse_mode: Option<&str>,
) -> Result<i64, String> {
    let text: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    let keyboard: Vec<Vec<serde_json::Value>> = buttons
        .into_iter()
        .map(|(label, data)| vec![serde_json::json!({ "text": label, "callback_data": data })])
        .collect();
    let mut body = serde_json::json!({ "chat_id": chat_id, "text": text });
    if !keyboard.is_empty() {
        body["reply_markup"] = serde_json::json!({ "inline_keyboard": keyboard });
    }
    if let Some(parse_mode) = parse_mode {
        if parse_mode != "HTML" {
            return Err("El formato del mensaje de Telegram no es valido.".to_string());
        }
        body["parse_mode"] = serde_json::Value::String(parse_mode.to_string());
    }
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .post(endpoint(token, "sendMessage")?)
        .json(&body)
        .send()
        .await
        .map_err(|_| "No se pudo enviar el mensaje a Telegram.".to_string())?;
    decode::<TelegramSentMessage>(response)
        .await
        .map(|message| message.message_id)
}

pub async fn edit_message(
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
    buttons: Vec<(String, String)>,
    parse_mode: Option<&str>,
) -> Result<(), String> {
    if chat_id == 0 {
        return Err("El identificador del chat de Telegram no es valido.".to_string());
    }
    if message_id <= 0 {
        return Err("El identificador del mensaje de Telegram no es valido.".to_string());
    }
    let text: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    if text.trim().is_empty() {
        return Err("El mensaje de Telegram no puede estar vacio.".to_string());
    }
    let keyboard: Vec<Vec<serde_json::Value>> = buttons
        .into_iter()
        .map(|(label, data)| vec![serde_json::json!({ "text": label, "callback_data": data })])
        .collect();
    let mut body =
        serde_json::json!({ "chat_id": chat_id, "message_id": message_id, "text": text });
    if !keyboard.is_empty() {
        body["reply_markup"] = serde_json::json!({ "inline_keyboard": keyboard });
    }
    if let Some(parse_mode) = parse_mode {
        if parse_mode != "HTML" {
            return Err("El formato del mensaje de Telegram no es valido.".to_string());
        }
        body["parse_mode"] = serde_json::Value::String(parse_mode.to_string());
    }
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .post(endpoint(token, "editMessageText")?)
        .json(&body)
        .send()
        .await
        .map_err(|_| "No se pudo actualizar el mensaje de Telegram.".to_string())?;
    decode::<serde_json::Value>(response).await.map(|_| ())
}

/// Sends a PNG image the agent made. As a photo first; when Telegram
/// rejects it (a photo is limited to 10 MB and 10000 px of width plus
/// height), as a file, which keeps it whole.
pub async fn send_photo(token: &str, chat_id: i64, file_name: &str, png: Vec<u8>) -> Result<i64, String> {
    if chat_id == 0 || png.is_empty() {
        return Err("La imagen para Telegram no es valida.".to_string());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| error.to_string())?;
    let mut last_error = "No se pudo enviar la imagen a Telegram.".to_string();
    for (method, field) in [("sendPhoto", "photo"), ("sendDocument", "document")] {
        let part = reqwest::multipart::Part::bytes(png.clone())
            .file_name(file_name.to_string())
            .mime_str("image/png")
            .map_err(|error| error.to_string())?;
        let form = reqwest::multipart::Form::new().text("chat_id", chat_id.to_string()).part(field, part);
        let response = match client.post(endpoint(token, method)?).multipart(form).send().await {
            Ok(response) => response,
            Err(_) => continue,
        };
        match decode::<TelegramSentMessage>(response).await {
            Ok(message) => return Ok(message.message_id),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// Deletes a message the bot sent (Telegram allows it for 48 hours).
pub async fn delete_message(token: &str, chat_id: i64, message_id: i64) -> Result<(), String> {
    if chat_id == 0 || message_id <= 0 {
        return Err("El mensaje de Telegram no es valido.".to_string());
    }
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .post(endpoint(token, "deleteMessage")?)
        .json(&serde_json::json!({ "chat_id": chat_id, "message_id": message_id }))
        .send()
        .await
        .map_err(|_| "No se pudo borrar el mensaje de Telegram.".to_string())?;
    decode::<serde_json::Value>(response).await.map(|_| ())
}

pub async fn download_audio(token: &str, audio: &TelegramAudio) -> Result<Vec<u8>, String> {
    if audio.file_id.is_empty() || audio.file_id.len() > 256 {
        return Err("El identificador del audio de Telegram no es valido.".to_string());
    }
    if audio.duration == 0 || audio.duration > MAX_AUDIO_DURATION_SECONDS {
        return Err("El audio de Telegram supera los quince minutos permitidos.".to_string());
    }
    if audio
        .file_size
        .is_some_and(|size| size == 0 || size > MAX_AUDIO_BYTES)
    {
        return Err("El audio de Telegram supera el tamaño permitido.".to_string());
    }
    if let Some(mime_type) = audio.mime_type.as_deref() {
        if !matches!(mime_type, "audio/ogg" | "audio/opus" | "application/ogg") {
            return Err("Por ahora Telegram solo admite notas de voz OGG/Opus.".to_string());
        }
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(endpoint(token, "getFile")?)
        .json(&serde_json::json!({ "file_id": audio.file_id }))
        .send()
        .await
        .map_err(|_| "No se pudo consultar el audio en Telegram.".to_string())?;
    let file = decode::<TelegramFile>(response).await?;
    if file
        .file_size
        .is_some_and(|size| size == 0 || size > MAX_AUDIO_BYTES)
    {
        return Err("El audio de Telegram supera el tamaño permitido.".to_string());
    }
    let file_path = file
        .file_path
        .filter(|path| {
            !path.is_empty()
                && path.len() <= 512
                && path
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
                && !path.split('/').any(|component| component == "..")
        })
        .ok_or_else(|| "Telegram devolvio una ruta de audio no valida.".to_string())?;
    let download_url = format!("{}/file/bot{}/{file_path}", api_base()?, token.trim());
    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|_| "No se pudo descargar el audio de Telegram.".to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_AUDIO_BYTES)
    {
        return Err("Telegram rechazo la descarga del audio.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Se interrumpio la descarga del audio de Telegram.".to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_AUDIO_BYTES {
        return Err("El audio descargado tiene un tamaño no valido.".to_string());
    }
    Ok(bytes.to_vec())
}

pub async fn download_photo(token: &str, photo: &TelegramPhoto) -> Result<Vec<u8>, String> {
    if photo.file_id.is_empty()
        || photo.file_id.len() > 256
        || photo.width == 0
        || photo.height == 0
        || u64::from(photo.width) * u64::from(photo.height) > MAX_PHOTO_PIXELS
    {
        return Err("La imagen de Telegram no es valida.".to_string());
    }
    if photo
        .file_size
        .is_some_and(|size| size == 0 || size > MAX_PHOTO_BYTES)
    {
        return Err("La imagen de Telegram supera el tamaño permitido.".to_string());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(endpoint(token, "getFile")?)
        .json(&serde_json::json!({ "file_id": photo.file_id }))
        .send()
        .await
        .map_err(|_| "No se pudo consultar la imagen en Telegram.".to_string())?;
    let file = decode::<TelegramFile>(response).await?;
    if file
        .file_size
        .is_some_and(|size| size == 0 || size > MAX_PHOTO_BYTES)
    {
        return Err("La imagen de Telegram supera el tamaño permitido.".to_string());
    }
    let file_path = file
        .file_path
        .filter(|path| {
            !path.is_empty()
                && path.len() <= 512
                && path
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
                && !path.split('/').any(|component| component == "..")
        })
        .ok_or_else(|| "Telegram devolvio una ruta de imagen no valida.".to_string())?;
    let download_url = format!("{}/file/bot{}/{file_path}", api_base()?, token.trim());
    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|_| "No se pudo descargar la imagen de Telegram.".to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_PHOTO_BYTES)
    {
        return Err("Telegram rechazo la descarga de la imagen.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Se interrumpio la descarga de la imagen de Telegram.".to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_PHOTO_BYTES {
        return Err("La imagen descargada tiene un tamaño no valido.".to_string());
    }
    Ok(bytes.to_vec())
}

pub async fn download_document(
    token: &str,
    document: &TelegramDocument,
) -> Result<Vec<u8>, String> {
    if document.file_id.is_empty() || document.file_id.len() > 256 {
        return Err("El identificador del documento de Telegram no es valido.".to_string());
    }
    let max_bytes = match document_kind(document) {
        Some(DocumentKind::Pdf) => MAX_DOCUMENT_BYTES,
        Some(DocumentKind::Image) => MAX_IMAGE_DOCUMENT_BYTES,
        None => return Err("Por ahora Telegram admite fotos, imágenes JPG o PNG y documentos PDF.".to_string()),
    };
    if document
        .file_size
        .is_some_and(|size| size == 0 || size > max_bytes)
    {
        return Err("El documento de Telegram supera el tamaño permitido.".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| "No se pudo iniciar la descarga del documento.".to_string())?;
    let response = client
        .post(endpoint(token, "getFile")?)
        .json(&serde_json::json!({ "file_id": document.file_id }))
        .send()
        .await
        .map_err(|_| "No se pudo consultar el documento en Telegram.".to_string())?;
    let file = decode::<TelegramFile>(response).await?;
    if file
        .file_size
        .is_some_and(|size| size == 0 || size > max_bytes)
    {
        return Err("El documento de Telegram supera el tamaño permitido.".to_string());
    }
    let file_path = file
        .file_path
        .filter(|path| {
            !path.is_empty()
                && path.len() <= 512
                && path
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
                && !path.split('/').any(|component| component == "..")
        })
        .ok_or_else(|| "Telegram devolvio una ruta de documento no valida.".to_string())?;
    let response = client
        .get(format!(
            "{}/file/bot{}/{file_path}",
            api_base()?,
            token.trim()
        ))
        .send()
        .await
        .map_err(|_| "No se pudo descargar el documento de Telegram.".to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > max_bytes)
    {
        return Err("Telegram rechazo la descarga del documento.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Se interrumpio la descarga del documento de Telegram.".to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        return Err("El documento descargado tiene un tamaño no valido.".to_string());
    }
    Ok(bytes.to_vec())
}

pub fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    let document = lopdf::Document::load_mem(bytes)
        .map_err(|_| "El documento de Telegram no es un PDF válido.".to_string())?;
    let page_numbers: Vec<u32> = document.get_pages().into_keys().collect();
    if page_numbers.is_empty() {
        return Err("El PDF de Telegram no contiene páginas.".to_string());
    }
    let mut text = String::new();
    for page_number in page_numbers {
        // A page whose fonts lopdf cannot decode is left out; the others
        // still reach the model.
        let Ok(page_text) = document.extract_text(&[page_number]) else { continue };
        text.push_str(&page_text);
        if text.len() > 120_000 {
            return Err("El texto extraído del PDF supera el límite permitido.".to_string());
        }
    }
    if text.trim().is_empty() {
        return Err("El PDF no contiene texto seleccionable.".to_string());
    }
    Ok(text)
}

pub async fn answer_callback(token: &str, callback_query_id: &str) -> Result<(), String> {
    let response = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .post(endpoint(token, "answerCallbackQuery")?)
        .json(&serde_json::json!({ "callback_query_id": callback_query_id }))
        .send()
        .await
        .map_err(|_| "No se pudo confirmar la accion en Telegram.".to_string())?;
    decode::<bool>(response).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{
        document_kind, edit_message, endpoint, send_message, DocumentKind, TelegramAudio, TelegramDocument,
        TelegramPhoto,
    };

    /// A Type0 font (2-byte codes) whose map has a 1-byte entry, as in
    /// Galicia's statements, keeps its text; a page lopdf cannot decode does
    /// not drop the others.
    #[test]
    fn pdf_text_survives_loose_and_broken_font_maps() {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let mut page = |text: &str, to_unicode: &[u8]| {
            let cmap = document.add_object(Stream::new(dictionary! {}, to_unicode.to_vec()));
            let descendant = document.add_object(dictionary! {
                "Type" => "Font",
                "Subtype" => "CIDFontType2",
                "BaseFont" => "Inter",
                "CIDSystemInfo" => dictionary! {
                    "Registry" => Object::string_literal("Adobe"),
                    "Ordering" => Object::string_literal("Identity"),
                    "Supplement" => 0,
                },
            });
            let font = document.add_object(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type0",
                "BaseFont" => "Inter",
                "Encoding" => "Identity-H",
                "DescendantFonts" => vec![descendant.into()],
                "ToUnicode" => cmap,
            });
            let codes = text.chars().map(|character| format!("{:04X}", character as u32)).collect::<String>();
            let content = document.add_object(Stream::new(dictionary! {}, format!("BT /F1 12 Tf 72 720 Td <{codes}> Tj ET").into_bytes()));
            document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            })
        };
        // The code space is 2 bytes but one entry maps a single byte: lopdf
        // 0.38 rejected the whole table and the page lost its text.
        let loose = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /TTX+0 def /CMapType 2 def\n1 begincodespacerange\n<0000><FFFF>\nendcodespacerange\n1 beginbfchar\n<40><0020>\nendbfchar\n1 beginbfrange\n<0020><007E><0020>\nendbfrange\nendcmap CMapName currentdict /CMap defineresource pop end end";
        let first = page("Total a pagar", loose);
        let second = page("Hoja rota", b"begincmap 1 beginbfchar <zz> endcmap");
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![first.into(), second.into()], "Count" => 2 }),
        );
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("pdf");
        let text = super::extract_pdf_text(&bytes).expect("text");
        assert!(text.contains("Total a pagar"), "{text:?}");
    }

    #[test]
    fn documents_are_pdfs_or_images_sent_as_files() {
        let document = |name: &str, mime: Option<&str>| TelegramDocument {
            file_id: "f".into(),
            file_name: Some(name.into()),
            mime_type: mime.map(str::to_string),
            file_size: Some(10),
        };
        assert_eq!(document_kind(&document("resumen.PDF", None)), Some(DocumentKind::Pdf));
        assert_eq!(document_kind(&document("x", Some("application/pdf"))), Some(DocumentKind::Pdf));
        assert_eq!(document_kind(&document("captura.png", Some("image/png"))), Some(DocumentKind::Image));
        assert_eq!(document_kind(&document("ticket.jpg", None)), Some(DocumentKind::Image));
        assert_eq!(document_kind(&document("foto.heic", Some("image/heic"))), None);
        assert_eq!(document_kind(&document("planilla.xlsx", Some("application/vnd.ms-excel"))), None);
    }

    #[test]
    fn endpoint_rejects_tokens_with_url_characters() {
        assert!(endpoint("https://example.com", "getMe").is_err());
        assert!(endpoint("123:abc/def", "getMe").is_err());
    }

    #[test]
    fn endpoint_accepts_standard_bot_token_shape() {
        assert_eq!(
            endpoint("123456:ABC_def-9", "getMe").unwrap(),
            "https://api.telegram.org/bot123456:ABC_def-9/getMe"
        );
    }

    #[test]
    fn telegram_send_response_exposes_message_id() {
        let message: super::TelegramSentMessage =
            serde_json::from_str(r#"{"message_id":42}"#).expect("deserialize sent message");
        assert_eq!(message.message_id, 42);
    }

    #[test]
    fn telegram_audio_accepts_bot_api_fields_and_serializes_camel_case() {
        let audio: TelegramAudio = serde_json::from_str(
            r#"{"file_id":"voice-1","duration":12,"mime_type":"audio/ogg","file_size":345}"#,
        )
        .expect("deserialize Telegram audio");
        let value = serde_json::to_value(audio).expect("serialize frontend audio");

        assert_eq!(value["fileId"], "voice-1");
        assert_eq!(value["mimeType"], "audio/ogg");
        assert_eq!(value["fileSize"], 345);
    }

    #[test]
    fn telegram_photo_accepts_bot_api_fields_and_serializes_camel_case() {
        let photo: TelegramPhoto = serde_json::from_str(
            r#"{"file_id":"photo-1","file_size":678,"width":1280,"height":720}"#,
        )
        .expect("deserialize Telegram photo");
        let value = serde_json::to_value(photo).expect("serialize frontend photo");

        assert_eq!(value["fileId"], "photo-1");
        assert_eq!(value["fileSize"], 678);
        assert_eq!(value["width"], 1280);
        assert_eq!(value["height"], 720);
    }

    #[test]
    fn telegram_document_accepts_pdf_fields_and_serializes_camel_case() {
        let document: TelegramDocument = serde_json::from_str(
            r#"{"file_id":"doc-1","file_name":"sueldo.pdf","mime_type":"application/pdf","file_size":678}"#,
        ).expect("deserialize Telegram document");
        let value = serde_json::to_value(document).expect("serialize frontend document");
        assert_eq!(value["fileId"], "doc-1");
        assert_eq!(value["fileName"], "sueldo.pdf");
        assert_eq!(value["mimeType"], "application/pdf");
    }

    #[test]
    fn telegram_message_validation_fails_before_network_requests() {
        futures::executor::block_on(async {
            assert!(edit_message("fixture-token", 0, 10, "estado", vec![], None)
                .await
                .is_err());
            assert!(edit_message("fixture-token", 42, 0, "estado", vec![], None)
                .await
                .is_err());
            assert!(edit_message("fixture-token", 42, 10, "   ", vec![], None)
                .await
                .is_err());
            assert!(
                edit_message("fixture-token", 42, 10, "estado", vec![], Some("Markdown"))
                    .await
                    .is_err()
            );
            assert!(
                send_message("fixture-token", 42, "estado", vec![], Some("Markdown"))
                    .await
                    .is_err()
            );
        });
    }
}
