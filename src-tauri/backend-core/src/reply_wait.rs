//! The wait before answering someone who writes in several messages: many
//! people send a request line by line instead of all at once. Telegram
//! (the Owner's requests), the WhatsApp auto replies and the AI tasks that
//! wait for a WhatsApp reply wait this long from the last message; a new
//! message starts the wait again, and once it ends they read every message
//! that arrived and answer them together.

use std::time::Duration;

/// The wait of a WhatsApp auto reply.
pub const WHATSAPP_WAIT: Duration = Duration::from_secs(60);
/// The wait of an AI task for a reply in a WhatsApp chat: a company's bot
/// sends its menu in a burst of messages within seconds, and a task talks
/// with it turn after turn.
pub const WHATSAPP_TASK_WAIT: Duration = Duration::from_secs(15);
/// The wait of a new Telegram request.
pub const TELEGRAM_WAIT: Duration = Duration::from_secs(10);
