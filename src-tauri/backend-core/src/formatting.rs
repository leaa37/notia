use super::protocol::ChannelResponse;

const ALLOWED_TAGS: &[&str] = &["b", "i", "u", "s", "code", "pre", "a"];

/// Builds both channel representations from the canonical Markdown response.
/// Telegram output is generated from scratch so model-provided HTML is never
/// copied into the transport payload.
pub fn channel_response(markdown: impl Into<String>) -> ChannelResponse {
    let markdown = markdown.into();
    ChannelResponse {
        telegram_html: markdown_to_telegram_html(&markdown),
        markdown,
        data: serde_json::Value::Null,
    }
}

pub fn escape_telegram_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Converts the supported Markdown subset to Telegram HTML. Unsupported
/// Markdown and all input HTML are emitted as escaped text.
pub fn markdown_to_telegram_html(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut in_fence = false;

    for (index, line) in markdown.lines().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                output.push_str("</pre>");
            } else {
                output.push_str("<pre>");
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            output.push_str(&escape_telegram_html(line));
            continue;
        }

        // An indented item keeps its indentation before the bullet.
        let body = line.trim_start();
        let line = match list_prefix(body) {
            (rest, Some(prefix)) => {
                output.push_str(&line[..line.len() - body.len()]);
                output.push_str(prefix);
                rest
            }
            (_, None) => line,
        };
        if let Some(heading) = heading_text(line) {
            output.push_str("<b>");
            output.push_str(&render_inline(heading));
            output.push_str("</b>");
        } else {
            output.push_str(&render_inline(line));
        }
    }

    if in_fence {
        output.push_str("</pre>");
    }
    output
}

fn list_prefix(value: &str) -> (&str, Option<&'static str>) {
    if let Some(rest) = value
        .strip_prefix("- ")
        .or_else(|| value.strip_prefix("* "))
    {
        return (rest, Some("• "));
    }
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && matches!(value.as_bytes().get(digits), Some(b'.' | b')')) {
        let start = digits + 1;
        if value.as_bytes().get(start) == Some(&b' ') {
            return (&value[start + 1..], Some("• "));
        }
    }
    (value, None)
}

/// The text of a `#` to `######` heading.
fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    (1..=6).contains(&hashes).then(|| line[hashes..].strip_prefix(' ')).flatten()
}

/// Where the emphasis `marker` opening `rest` closes, counted from its
/// content. The content neither starts nor ends with a space, so «2 * 3»
/// is no emphasis, and a single `*` is not half of a `**`.
fn emphasis_end(rest: &str, marker: &str) -> Option<usize> {
    let inner = &rest[marker.len()..];
    if inner.is_empty() || inner.starts_with(char::is_whitespace) {
        return None;
    }
    let mut from = 0;
    while let Some(found) = inner[from..].find(marker) {
        let at = from + found;
        let after_space = inner[..at].ends_with(char::is_whitespace);
        let half_of_double = marker.len() == 1 && inner[at + 1..].starts_with(marker);
        if at > 0 && !after_space && !half_of_double {
            return Some(at);
        }
        from = at + marker.len();
    }
    None
}

fn render_inline(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let rest = &value[index..];
        if let Some(end) = rest.strip_prefix('`').and_then(|rest| rest.find('`')) {
            let content = &rest[1..end + 1];
            output.push_str("<code>");
            output.push_str(&escape_telegram_html(content));
            output.push_str("</code>");
            index += end + 2;
            continue;
        }

        if let Some((open, close, inner, consumed)) = html_inline_tag(rest) {
            output.push_str(open);
            output.push_str(&if open == "<code>" { escape_telegram_html(inner) } else { render_inline(inner) });
            output.push_str(close);
            index += consumed;
            continue;
        }
        if let Some(consumed) = html_line_break(rest) {
            output.push('\n');
            index += consumed;
            continue;
        }

        if let Some((text, url, consumed)) = markdown_link(rest) {
            output.push_str("<a href=\"");
            output.push_str(&escape_attribute(url));
            output.push_str("\">");
            output.push_str(&render_inline(text));
            output.push_str("</a>");
            index += consumed;
            continue;
        }

        let marker = if rest.starts_with("***") {
            Some(("***", "<b><i>", "</i></b>"))
        } else if rest.starts_with("**") {
            Some(("**", "<b>", "</b>"))
        } else if rest.starts_with("__") {
            Some(("__", "<b>", "</b>"))
        } else if rest.starts_with('*') {
            Some(("*", "<i>", "</i>"))
        } else if rest.starts_with('_') {
            Some(("_", "<i>", "</i>"))
        } else if rest.starts_with("~~") {
            Some(("~~", "<s>", "</s>"))
        } else {
            None
        };
        if let Some((marker, open, close)) = marker {
            if let Some(end) = emphasis_end(rest, marker) {
                output.push_str(open);
                output.push_str(&render_inline(&rest[marker.len()..marker.len() + end]));
                output.push_str(close);
                index += marker.len() * 2 + end;
                continue;
            }
            // A bold or strike that never closes on its line (it spans two
            // lines, or the model left it open) would show its asterisks.
            if marker.len() >= 2 && marker != "__" {
                index += marker.len();
                continue;
            }
        }

        let character = rest.chars().next().expect("index is on a character");
        output.push_str(&escape_telegram_html(&character.to_string()));
        index += character.len_utf8();
    }
    output
}

/// A formatting tag the model wrote as HTML (`<b>texto</b>`), without
/// attributes and closed on the same line: the Telegram tags to emit, its
/// content and the bytes it spans. The tags are rebuilt, never copied, and
/// the content is rendered like any other text.
fn html_inline_tag(value: &str) -> Option<(&'static str, &'static str, &str, usize)> {
    const TAGS: [(&str, &str, &str); 10] = [
        ("b", "<b>", "</b>"),
        ("strong", "<b>", "</b>"),
        ("i", "<i>", "</i>"),
        ("em", "<i>", "</i>"),
        ("u", "<u>", "</u>"),
        ("ins", "<u>", "</u>"),
        ("s", "<s>", "</s>"),
        ("del", "<s>", "</s>"),
        ("strike", "<s>", "</s>"),
        ("code", "<code>", "</code>"),
    ];
    let name_end = value.strip_prefix('<')?.find('>')?;
    let name = value[1..name_end + 1].trim().to_ascii_lowercase();
    let (_, open, close) = TAGS.iter().find(|(tag, ..)| *tag == name)?;
    let start = name_end + 2;
    let closing = format!("</{name}>");
    let end = value[start..].to_ascii_lowercase().find(&closing)?;
    Some((open, close, &value[start..start + end], start + end + closing.len()))
}

/// Bytes of a `<br>`, `<br/>` or `<br />` at the start of `value`.
fn html_line_break(value: &str) -> Option<usize> {
    ["<br>", "<br/>", "<br />"]
        .iter()
        .find(|tag| value.get(..tag.len()).is_some_and(|start| start.eq_ignore_ascii_case(tag)))
        .map(|tag| tag.len())
}

fn markdown_link(value: &str) -> Option<(&str, &str, usize)> {
    let text_end = value.strip_prefix('[')?.find(']')?;
    let text = &value[1..text_end + 1];
    let after_text = &value[text_end + 2..];
    let url_end = after_text.strip_prefix('(')?.find(')')?;
    let url = &after_text[1..url_end + 1];
    if !allowed_url(url) {
        return None;
    }
    Some((text, url, text_end + url_end + 4))
}

fn allowed_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '"' | '<' | '>'))
}

fn escape_attribute(value: &str) -> String {
    escape_telegram_html(value).replace('"', "&quot;")
}

#[allow(dead_code)]
pub fn allowed_telegram_tags() -> &'static [&'static str] {
    ALLOWED_TAGS
}

#[cfg(test)]
mod tests {
    use super::{channel_response, escape_telegram_html, markdown_to_telegram_html};

    #[test]
    fn escapes_input_html_and_special_characters() {
        assert_eq!(
            markdown_to_telegram_html("<script>alert('&')</script>"),
            "&lt;script&gt;alert('&amp;')&lt;/script&gt;"
        );
        assert_eq!(escape_telegram_html("<&>"), "&lt;&amp;&gt;");
    }

    #[test]
    fn renders_only_the_supported_markdown_subset() {
        assert_eq!(
            markdown_to_telegram_html("# **Título**\n- `código`\n[fuente](https://example.com)"),
            "<b><b>Título</b></b>\n• <code>código</code>\n<a href=\"https://example.com\">fuente</a>"
        );
        assert_eq!(
            markdown_to_telegram_html("[unsafe](javascript:alert(1))"),
            "[unsafe](javascript:alert(1))"
        );
    }

    #[test]
    fn formatting_tags_written_as_html_become_formatting() {
        assert_eq!(
            markdown_to_telegram_html("• <b>Google Play:</b> vence el <B>3/10</B>, <i>Face Yoga</i> y <code>a<b</code><br/>fin"),
            "• <b>Google Play:</b> vence el <b>3/10</b>, <i>Face Yoga</i> y <code>a&lt;b</code>\nfin"
        );
        assert_eq!(markdown_to_telegram_html("<strong>x</strong> <em>y</em>"), "<b>x</b> <i>y</i>");
        assert_eq!(markdown_to_telegram_html("<b onclick=\"x\">a</b> <b>sin cierre"), "&lt;b onclick=\"x\"&gt;a&lt;/b&gt; &lt;b&gt;sin cierre");
    }

    #[test]
    fn short_reminders_never_show_their_markdown() {
        assert_eq!(markdown_to_telegram_html("⏰ **Recordatorio:** sacar la basura"), "⏰ <b>Recordatorio:</b> sacar la basura");
        assert_eq!(markdown_to_telegram_html("**Recordatorio\nsigue**"), "Recordatorio\nsigue");
        assert_eq!(markdown_to_telegram_html("***Importante***"), "<b><i>Importante</i></b>");
        assert_eq!(markdown_to_telegram_html("Precio 2 * 3 y **bold**"), "Precio 2 * 3 y <b>bold</b>");
        assert_eq!(markdown_to_telegram_html("#### Hoy\n  - **sub** item\n1) **Pagar** luz"), "<b>Hoy</b>\n  • <b>sub</b> item\n• <b>Pagar</b> luz");
        assert_eq!(markdown_to_telegram_html("*Ana* y **Luis**"), "<i>Ana</i> y <b>Luis</b>");
        assert_eq!(markdown_to_telegram_html("#hashtag"), "#hashtag");
    }

    #[test]
    fn returns_markdown_and_safe_telegram_variants() {
        let response = channel_response("**respuesta**".to_string());
        assert_eq!(response.markdown, "**respuesta**");
        assert_eq!(response.telegram_html, "<b>respuesta</b>");
    }
}
