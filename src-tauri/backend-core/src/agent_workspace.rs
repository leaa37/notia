//! Content rules of the library's `.agent` workspace: the managed default
//! rules block, rules added by the agent, persistent memories, the agent's
//! own thoughts, the person's biography and way of talking, the people
//! they mention, the confidential context of every agent file and prompt names. The adapter
//! only reads and writes the files.

use crate::prompt::{strip_frontmatter, DEFAULT_AGENT_RULES};

pub const AGENT_DIRECTORY: &str = ".agent";
pub const PROMPTS_DIRECTORY: &str = ".agent/promps";
pub const MEMORY_DIRECTORY: &str = ".agent/memory";
pub const AGENT_FOLDERS: [&str; 6] = [
    ".agent",
    ".agent/promps",
    ".agent/dynamics",
    ".agent/skills",
    ".agent/memory",
    ".agent/scripts",
];
pub const RULES_PATH: &str = ".agent/memory/rules.md";
pub const MEMORY_PATH: &str = ".agent/memory/memory.md";
/// The agent's own working notes: what it noticed, told, asked or proposed.
pub const THOUGHTS_PATH: &str = ".agent/memory/thoughts.md";
/// The person's biography, built by the agent from what they tell it.
pub const BIOGRAPHY_PATH: &str = ".agent/memory/biography.md";
/// How the person talks, so the agent talks alike.
pub const TALK_PATH: &str = ".agent/memory/talk.md";
/// Who is who among the people the person mentions.
pub const PEOPLE_PATH: &str = ".agent/memory/people.md";
pub const DEFAULT_PROMPT_FILE: &str = "default.md";
pub const DEFAULT_PROMPT_PATH: &str = ".agent/promps/default.md";
pub const MAX_MEMORIES: usize = 1_000;
pub const MAX_RULE_CHARS: usize = 8_000;
/// Characters of every memory together; the prompt reads at most 250,000
/// per agent file, so the memory file must stay below that.
pub const MAX_MEMORY_CHARS: usize = 150_000;
/// Budget a new memory must fit in before it is added.
pub const MEMORY_LIMIT: ItemBudget = ItemBudget { items: MAX_MEMORIES, chars: MAX_MEMORY_CHARS };
/// What a full memory is rewritten down to, leaving room for new memories.
pub const MEMORY_TARGET: ItemBudget = ItemBudget { items: 800, chars: 120_000 };
pub const MAX_THOUGHT_CHARS: usize = 1_000;
/// Hard limit of the thoughts file: reaching it forces a rewrite.
pub const THOUGHTS_LIMIT: ItemBudget = ItemBudget { items: 500, chars: 100_000 };
/// What every reorganization of the thoughts keeps them within.
pub const THOUGHTS_TARGET: ItemBudget = ItemBudget { items: 400, chars: 80_000 };
/// Rules the agent added (the managed default block is not counted):
/// reaching the limit forces a rewrite, like the memory.
pub const RULES_LIMIT: ItemBudget = ItemBudget { items: 300, chars: 40_000 };
/// What a full rules block is rewritten down to.
pub const RULES_TARGET: ItemBudget = ItemBudget { items: 240, chars: 32_000 };

/// A list file of the agent kept like the memory: one item per line, a
/// version marker, a hard limit that forces a rewrite and the size every
/// review brings it back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentListFile {
    pub path: &'static str,
    marker: &'static str,
    pub limit: ItemBudget,
    pub target: ItemBudget,
    /// Longest single item.
    pub max_item_chars: usize,
}

impl AgentListFile {
    /// File body for `items`, without duplicates.
    pub fn render(&self, items: &[String]) -> String {
        render_list(self.marker, &dedupe_items(items.iter().cloned()))
    }
}

/// Longest story the biography may hold: a longer one is rewritten.
pub const BIOGRAPHY_STORY_LIMIT: usize = 80_000;
/// What a biography grown past its limit is rewritten down to.
pub const BIOGRAPHY_STORY_TARGET: usize = 64_000;
/// Facts waiting to be told in the story.
pub const BIOGRAPHY_NOTES_LIMIT: ItemBudget = ItemBudget { items: 200, chars: 30_000 };
/// Longest single fact the agent may add.
pub const MAX_BIOGRAPHY_NOTE_CHARS: usize = 1_000;
const BIOGRAPHY_MARKER: &str = "<!-- NOTIA_AGENT_BIOGRAPHY_VERSION:2 -->";
/// The first biographies were a plain list of facts.
const BIOGRAPHY_LIST_MARKER: &str = "<!-- NOTIA_AGENT_BIOGRAPHY_VERSION:1 -->";
const BIOGRAPHY_NOTES_START: &str = "<!-- NOTIA_BIOGRAPHY_NOTES_START -->";
const BIOGRAPHY_NOTES_END: &str = "<!-- NOTIA_BIOGRAPHY_NOTES_END -->";
const BIOGRAPHY_NOTES_HEADING: &str = "### Datos por incorporar";

/// The person's biography, told like a book: a Markdown story with a title
/// and a chapter per stage of their life, and the facts learned since the
/// story was last written, which the model weaves in afterwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Biography {
    pub story: String,
    pub notes: Vec<String>,
}

/// Result of adding one fact to the biography.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BiographyAppend {
    Added(Biography),
    /// Empty or already waiting.
    Unchanged,
    /// No room for more facts: the story must be written first.
    Full,
}

impl Biography {
    /// The biography of a file body. A list from the first version becomes
    /// facts to tell; text outside the facts block is the story.
    pub fn parse(body: &str) -> Self {
        let body = body.replace("\r\n", "\n");
        if body.contains(BIOGRAPHY_LIST_MARKER) {
            return Self { story: String::new(), notes: dedupe_items(parse_memory_items(&body)) };
        }
        let (story, notes) = match (body.find(BIOGRAPHY_NOTES_START), body.find(BIOGRAPHY_NOTES_END)) {
            (Some(start), Some(end)) if end > start => (
                format!("{}\n{}", &body[..start], &body[end + BIOGRAPHY_NOTES_END.len()..]),
                parse_memory_items(&body[start + BIOGRAPHY_NOTES_START.len()..end]),
            ),
            _ => (body.clone(), Vec::new()),
        };
        let story = story
            .lines()
            .filter(|line| !line.trim_start().starts_with("<!-- NOTIA_"))
            .collect::<Vec<_>>()
            .join("\n");
        Self { story: collapse_blank_lines(story.trim()), notes: dedupe_items(notes) }
    }

    /// File body: the version marker, the story and, when there are any,
    /// the facts still to tell.
    pub fn render(&self) -> String {
        let mut parts = vec![BIOGRAPHY_MARKER.to_string()];
        let story = collapse_blank_lines(self.story.trim());
        if !story.is_empty() {
            parts.push(story);
        }
        let notes = dedupe_items(self.notes.iter().cloned());
        if !notes.is_empty() {
            let mut block = vec![BIOGRAPHY_NOTES_START.to_string(), BIOGRAPHY_NOTES_HEADING.to_string(), String::new()];
            block.extend(notes.iter().map(|note| format!("- {note}")));
            block.push(BIOGRAPHY_NOTES_END.to_string());
            parts.push(block.join("\n"));
        }
        format!("{}\n", parts.join("\n\n"))
    }

    /// The biography with one more fact to tell.
    pub fn with_note(&self, note: &str) -> BiographyAppend {
        let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
        if note.is_empty() || self.notes.iter().any(|known| known.eq_ignore_ascii_case(&note)) {
            return BiographyAppend::Unchanged;
        }
        if !BIOGRAPHY_NOTES_LIMIT.fits_one_more(&self.notes, &note) {
            return BiographyAppend::Full;
        }
        let mut next = self.clone();
        next.notes.push(note);
        BiographyAppend::Added(next)
    }

    /// Whether the story must be written again: facts are waiting, or it
    /// grew past its target.
    pub fn needs_writing(&self) -> bool {
        !self.notes.is_empty() || self.story.chars().count() > BIOGRAPHY_STORY_TARGET
    }
}

/// Text with runs of blank lines collapsed to one.
fn collapse_blank_lines(text: &str) -> String {
    let mut lines = Vec::<&str>::new();
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() && lines.last().is_some_and(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// How the person talks: short observations of their style. Small on
/// purpose: it steers every answer.
pub const TALK: AgentListFile = AgentListFile {
    path: TALK_PATH,
    marker: "<!-- NOTIA_AGENT_TALK_VERSION:1 -->",
    limit: ItemBudget { items: 60, chars: 8_000 },
    target: ItemBudget { items: 40, chars: 6_000 },
    max_item_chars: 400,
};

/// The people the person mentions: one line per person, `Nombre — quién es`.
pub const PEOPLE: AgentListFile = AgentListFile {
    path: PEOPLE_PATH,
    marker: "<!-- NOTIA_AGENT_PEOPLE_VERSION:1 -->",
    limit: ItemBudget { items: 500, chars: 60_000 },
    target: ItemBudget { items: 400, chars: 48_000 },
    max_item_chars: 600,
};
/// Longest name of a person.
pub const MAX_PERSON_NAME_CHARS: usize = 80;
const PERSON_SEPARATOR: &str = " — ";

/// The `Nombre — quién es` line of a person, with whitespace collapsed.
/// `None` when a part is missing or it does not fit in `PEOPLE`.
pub fn person_entry(name: &str, about: &str) -> Option<String> {
    let collapse = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = collapse(name.trim().trim_end_matches([':', '-', '—']));
    let about = collapse(about);
    let entry = format!("{name}{PERSON_SEPARATOR}{about}");
    let valid = !name.is_empty()
        && !about.is_empty()
        && name.chars().count() <= MAX_PERSON_NAME_CHARS
        && entry.chars().count() <= PEOPLE.max_item_chars;
    valid.then_some(entry)
}

/// A line of `people.md` in its canonical form: `Nombre — quién es`, also
/// from `Nombre: quién es` or `Nombre - quién es`, with Markdown bold
/// removed from the name. `None` when it does not name someone.
pub fn parse_person_entry(line: &str) -> Option<String> {
    let line = line.replace("**", "");
    let line = list_item(&line);
    let (index, separator) = [PERSON_SEPARATOR, " - ", ": "]
        .into_iter()
        .filter_map(|separator| line.find(separator).map(|index| (index, separator)))
        .min_by_key(|(index, _)| *index)?;
    person_entry(&line[..index], &line[index + separator.len()..])
}

const RULES_START: &str = "<!-- NOTIA_DEFAULT_RULES_START -->";
const RULES_END: &str = "<!-- NOTIA_DEFAULT_RULES_END -->";
const IA_RULES_START: &str = "<!-- NOTIA_IA_RULES_START -->";
const IA_RULES_END: &str = "<!-- NOTIA_IA_RULES_END -->";
const MEMORY_VERSION_MARKER: &str = "<!-- NOTIA_AGENT_MEMORY_VERSION:1 -->";
const THOUGHTS_VERSION_MARKER: &str = "<!-- NOTIA_AGENT_THOUGHTS_VERSION:1 -->";
const CONFIDENTIAL_CONTEXT_LINE: &str = "contexto: \"#Confidencial\"";
const SYSTEM_CONTEXT_LINE: &str = "contexto: \"#System\"";

/// The files only the AI or Notia writes: those of `.agent/memory`
/// (through the AI's tools, the reflection after each turn and the
/// organization of its files) and the guide of `.agent/scripts`, which
/// Notia keeps as it ships. Their context is `#System`: the person reads
/// them but never edits, renames, moves or deletes them by hand, nor the
/// folders that hold them.
pub const SYSTEM_FILES: [&str; 8] =
    [RULES_PATH, MEMORY_PATH, THOUGHTS_PATH, BIOGRAPHY_PATH, TALK_PATH, PEOPLE_PATH, IMPROVE_PATH, crate::agent_scripts::README_PATH];

/// Whether `path` (a logical path of the library) is one of `SYSTEM_FILES`.
pub fn is_system_file(path: &str) -> bool {
    let path = path.trim().trim_matches('/');
    SYSTEM_FILES.iter().any(|file| file.eq_ignore_ascii_case(path))
}

/// Whether `entry` is a system file or a folder that holds one, such as
/// `.agent` or `.agent/memory`.
pub fn holds_system_files(entry: &str) -> bool {
    let entry = entry.trim().trim_matches('/').to_lowercase();
    !entry.is_empty()
        && SYSTEM_FILES.iter().any(|file| {
            let file = file.to_lowercase();
            file == entry || file.starts_with(&format!("{entry}/"))
        })
}

/// How many list items, and how many characters in total, a list may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemBudget {
    pub items: usize,
    pub chars: usize,
}

impl ItemBudget {
    /// Whether `items` fit in the budget.
    pub fn fits(&self, items: &[String]) -> bool {
        items.len() <= self.items && total_chars(items) <= self.chars
    }

    /// Whether one more item of `extra` characters still fits.
    pub fn fits_one_more(&self, items: &[String], extra: &str) -> bool {
        items.len() < self.items && total_chars(items) + extra.chars().count() <= self.chars
    }
}

fn total_chars(items: &[String]) -> usize {
    items.iter().map(|item| item.chars().count()).sum()
}

/// Rules body with the current managed default block and an (possibly
/// empty) block for rules added by the agent.
pub fn ensure_default_rules(content: &str) -> String {
    let defaults = DEFAULT_AGENT_RULES.trim();
    let content = match (content.find(RULES_START), content.find(RULES_END)) {
        (Some(start), Some(end)) if end > start => format!(
            "{}{}{}",
            &content[..start],
            defaults,
            &content[end + RULES_END.len()..]
        )
        .trim()
        .to_string(),
        _ => [defaults, content.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    if content.contains(IA_RULES_START) && content.contains(IA_RULES_END) {
        content
    } else {
        format!("{content}\n\n{IA_RULES_START}\n{IA_RULES_END}")
    }
}

/// Byte range of the agent rules block inside an ensured rules body.
fn ia_block(content: &str) -> (usize, usize) {
    let start = content.find(IA_RULES_START).map_or(0, |index| index + IA_RULES_START.len());
    let end = content[start..].find(IA_RULES_END).map_or(content.len(), |index| start + index);
    (start, end)
}

fn list_item(line: &str) -> &str {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix('-'))
        .or_else(|| trimmed.strip_prefix('*'))
        .unwrap_or(trimmed)
        .trim()
}

fn with_ia_rules(content: &str, rules: &[String]) -> String {
    let (start, end) = ia_block(content);
    let block = rules.iter().map(|rule| format!("- {rule}")).collect::<Vec<_>>().join("\n");
    format!(
        "{}\n{}{}{}",
        &content[..start],
        block,
        if block.is_empty() { "" } else { "\n" },
        &content[end..]
    )
}

/// Rules added by the agent, without list markers.
pub fn ia_rules(content: &str) -> Vec<String> {
    let ensured = ensure_default_rules(content);
    let (start, end) = ia_block(&ensured);
    ensured[start..end]
        .lines()
        .map(list_item)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Replaces the agent rules block, dropping empty and repeated rules.
pub fn replace_ia_rules(content: &str, rules: &[String]) -> String {
    let mut unique = Vec::<String>::new();
    for rule in rules.iter().map(|rule| rule.split_whitespace().collect::<Vec<_>>().join(" ")) {
        if !rule.is_empty() && !unique.iter().any(|known| known.eq_ignore_ascii_case(&rule)) {
            unique.push(rule);
        }
    }
    with_ia_rules(&ensure_default_rules(content), &unique)
}

/// Result of adding one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAppend {
    /// The rules body with the rule added.
    Added(String),
    /// Empty or already present.
    Unchanged,
    /// The rules block has no room: it must be rewritten first.
    Full,
}

/// Adds one rule within `RULES_LIMIT`.
pub fn append_rule(content: &str, rule: &str) -> RuleAppend {
    let rule = rule.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rules = ia_rules(content);
    if rule.is_empty() || rules.iter().any(|known| known.eq_ignore_ascii_case(&rule)) {
        return RuleAppend::Unchanged;
    }
    if !RULES_LIMIT.fits_one_more(&rules, &rule) {
        return RuleAppend::Full;
    }
    rules.push(rule);
    RuleAppend::Added(with_ia_rules(&ensure_default_rules(content), &rules))
}

/// The rules body in its canonical form: the current managed block and the
/// agent rules one per line, without empty or repeated rules.
pub fn canonical_rules(content: &str) -> String {
    replace_ia_rules(content, &ia_rules(content))
}

fn starts_with_word(text: &str, prefix: &str) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().next().is_none_or(|next| !next.is_alphanumeric()))
}

/// Identity, preferences and personal context belong in memory, not rules.
pub fn is_likely_personal_memory(value: &str) -> bool {
    const PREFIXES: [&str; 14] = [
        "el usuario", "la usuaria", "mi nombre", "me llamo", "se llama", "su nombre", "trabajo",
        "trabaja", "vive", "le gusta", "prefiere", "esta trabajando", "esta arreglando", "tiene",
    ];
    let normalized = fold_accents(&list_item(value).to_lowercase());
    PREFIXES.iter().any(|prefix| starts_with_word(&normalized, prefix))
}

/// Runtime corrections that older versions persisted as rules by mistake.
pub fn is_internal_agent_correction(value: &str) -> bool {
    const MARKERS: [&str; 5] = [
        "ninguna mutacion financiera se ejecuto",
        "la respuesta anuncia una accion pendiente pero no solicita herramientas",
        "detectaste un ticket recibido por telegram, pero aun no fue persistido",
        "no hagas la pregunta financiera como texto final",
        "la respuesta anterior no separo todos los tickets recuperados",
    ];
    let normalized = fold_accents(&value.to_lowercase());
    MARKERS.iter().any(|marker| normalized.contains(marker))
}

fn fold_accents(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// Moves personal facts out of the agent rules and drops internal
/// corrections. Returns the rules body and the facts to keep as memories.
pub fn migrate_misclassified_rules(content: &str) -> (String, Vec<String>) {
    let rules = ia_rules(content);
    let memories = rules.iter().filter(|rule| is_likely_personal_memory(rule)).cloned().collect();
    let retained = rules
        .into_iter()
        .filter(|rule| !is_likely_personal_memory(rule) && !is_internal_agent_correction(rule))
        .collect::<Vec<_>>();
    (with_ia_rules(&ensure_default_rules(content), &retained), memories)
}

/// Memory items of a memory file body (list markers, headings and comments
/// are ignored).
pub fn parse_memory_items(body: &str) -> Vec<String> {
    body.lines()
        .map(list_item)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("<!--"))
        .map(str::to_string)
        .collect()
}

/// Case-insensitive union keeping the latest spelling, in first-seen order.
pub fn dedupe_items(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut merged = Vec::<String>::new();
    for item in items.into_iter().map(|item| item.split_whitespace().collect::<Vec<_>>().join(" ")) {
        if item.is_empty() {
            continue;
        }
        match merged.iter_mut().find(|known| known.eq_ignore_ascii_case(&item)) {
            Some(known) => *known = item,
            None => merged.push(item),
        }
    }
    merged
}

/// Case-insensitive union keeping the latest spelling, capped.
pub fn merge_memories(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut merged = dedupe_items(items);
    merged.truncate(MAX_MEMORIES);
    merged
}

fn render_list(marker: &str, items: &[String]) -> String {
    let mut lines = vec![marker.to_string(), String::new()];
    lines.extend(items.iter().map(|item| format!("- {item}")));
    lines.push(String::new());
    lines.join("\n")
}

/// Memory file body for `memories`.
pub fn render_memories(memories: &[String]) -> String {
    render_list(MEMORY_VERSION_MARKER, memories)
}

/// Errors, missing tools and improvements the agent finds while it works,
/// for the person who develops Notia. The agent writes it and never reads
/// it: it is not part of any prompt nor of the organization of the agent
/// files.
pub const IMPROVE_PATH: &str = ".agent/memory/improve.md";
const IMPROVE_MARKER: &str = "<!-- NOTIA_AGENT_IMPROVE_VERSION:1 -->";
const IMPROVE_HEADER: &str = "# Mejoras para Notia\n\nLas anota el asistente cuando una herramienta falla, le falta una herramienta o algo de Notia podría funcionar mejor. No forma parte de su contexto.";
/// Longest description of one improvement.
pub const MAX_IMPROVEMENT_CHARS: usize = 800;
/// The file keeps the newest entries within this budget; older ones go.
pub const IMPROVE_LIMIT: ItemBudget = ItemBudget { items: 300, chars: 150_000 };
const IMPROVEMENT_SEPARATOR: &str = " · ";

/// What kind of improvement an entry of `improve.md` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImprovementKind {
    /// A tool failed or answered wrong through no fault of the request.
    Error,
    /// A tool the agent needed and does not have.
    Tool,
    /// Something that could work better.
    Improvement,
}

impl ImprovementKind {
    /// The kind a tool argument or a reflection names; anything else is an
    /// improvement.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "error" | "errores" | "fallo" | "bug" => Self::Error,
            "herramienta" | "herramientas" | "tool" | "falta" | "herramienta faltante" => Self::Tool,
            _ => Self::Improvement,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Tool => "herramienta",
            Self::Improvement => "mejora",
        }
    }
}

/// The `fecha · tipo · descripción` line of an improvement, with
/// whitespace collapsed. `None` when the description is empty or too long.
pub fn improvement_entry(now_label: &str, kind: ImprovementKind, text: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = text.chars().count();
    (length > 0 && length <= MAX_IMPROVEMENT_CHARS)
        .then(|| format!("{now_label}{IMPROVEMENT_SEPARATOR}{}{IMPROVEMENT_SEPARATOR}{text}", kind.label()))
}

/// The kind and description of an entry, without its date.
fn improvement_identity(entry: &str) -> &str {
    entry.split_once(IMPROVEMENT_SEPARATOR).map_or(entry, |(_, rest)| rest).trim()
}

/// The entries of an `improve.md` body: its `- ` lines.
pub fn parse_improvements(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

/// The entries with one more, dropping the oldest ones that no longer fit.
/// `None` when the same kind and description is already there.
pub fn with_improvement(entries: &[String], entry: String) -> Option<Vec<String>> {
    let identity = improvement_identity(&entry);
    if entries.iter().any(|known| improvement_identity(known).eq_ignore_ascii_case(identity)) {
        return None;
    }
    let mut next = entries.to_vec();
    next.push(entry);
    while !IMPROVE_LIMIT.fits(&next) && next.len() > 1 {
        next.remove(0);
    }
    Some(next)
}

/// `improve.md` body for `entries`, oldest first.
pub fn render_improvements(entries: &[String]) -> String {
    let mut lines = vec![IMPROVE_MARKER.to_string(), IMPROVE_HEADER.to_string(), String::new()];
    lines.extend(entries.iter().map(|entry| format!("- {entry}")));
    lines.push(String::new());
    lines.join("\n")
}

/// Thoughts file body for `thoughts`, without duplicates.
pub fn render_thoughts(thoughts: &[String]) -> String {
    render_list(THOUGHTS_VERSION_MARKER, &dedupe_items(thoughts.iter().cloned()))
}

/// A thought dated with the local time `now_label` (`AAAA-MM-DD HH:MM`),
/// with its whitespace collapsed. `None` when it is empty or too long.
pub fn stamp_thought(now_label: &str, text: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = text.chars().count();
    (length > 0 && length <= MAX_THOUGHT_CHARS).then(|| format!("[{now_label}] {text}"))
}

/// The text of a thought without its `[fecha]` prefix.
pub fn thought_text(thought: &str) -> &str {
    let trimmed = thought.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .map_or(trimmed, |(_, text)| text.trim())
}

/// Thoughts with `stamped` added last. A thought that says the same (apart
/// from its date, or in almost the same words) is replaced, so a repeated
/// note only refreshes its date.
pub fn with_thought(thoughts: &[String], stamped: String) -> Vec<String> {
    let text = thought_text(&stamped).to_string();
    let mut next = thoughts
        .iter()
        .filter(|known| !same_thought(thought_text(known), &text))
        .cloned()
        .collect::<Vec<_>>();
    next.push(stamped);
    next
}

/// Items within `budget`, without the oldest ones that do not fit; the
/// last item always stays. Thoughts (working notes) and talk traits (a
/// newer one wins a contradiction) are kept oldest first: when the model
/// could not bring a full file back to its size, the oldest go, so a new
/// item is never lost. Returns the items kept and how many were dropped.
pub fn without_oldest(mut thoughts: Vec<String>, budget: ItemBudget) -> (Vec<String>, usize) {
    let mut chars = total_chars(&thoughts);
    let mut dropped = 0;
    while thoughts.len() - dropped > 1 && (thoughts.len() - dropped > budget.items || chars > budget.chars) {
        chars -= thoughts[dropped].chars().count();
        dropped += 1;
    }
    thoughts.drain(..dropped);
    (thoughts, dropped)
}

/// Share of words two thoughts must have in common to be the same note.
const SAME_THOUGHT_SHARE: f64 = 0.8;

/// Two thoughts say the same: equal text, or at least 80 % of their words
/// in common (without accents, connectors or plurals).
fn same_thought(left: &str, right: &str) -> bool {
    if left.trim().eq_ignore_ascii_case(right.trim()) {
        return true;
    }
    let left = crate::text_search::query_terms(left);
    let right = crate::text_search::query_terms(right);
    let shared = left.iter().filter(|word| right.contains(word)).count();
    let union = left.len() + right.len() - shared;
    union > 0 && shared as f64 >= union as f64 * SAME_THOUGHT_SHARE
}

/// Document whose frontmatter marks it as confidential; the body is kept.
pub fn with_confidential_context(source: &str) -> String {
    with_context_line(source, CONFIDENTIAL_CONTEXT_LINE)
}

/// Document whose frontmatter marks it as `#System`; the body is kept.
pub fn with_system_context(source: &str) -> String {
    with_context_line(source, SYSTEM_CONTEXT_LINE)
}

/// An agent document with its context: `#System` for the files only the AI
/// writes, `#Confidencial` for the rest (prompts, skills, dynamics).
pub fn with_agent_context(path: &str, source: &str) -> String {
    if is_system_file(path) { with_system_context(source) } else { with_confidential_context(source) }
}

fn with_context_line(source: &str, context_line: &str) -> String {
    let normalized = source.replace("\r\n", "\n");
    let normalized = normalized.strip_prefix('\u{feff}').unwrap_or(&normalized);
    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---").filter(|end| {
            let after = &rest[end + 4..];
            after.is_empty() || after.starts_with('\n')
        }) {
            let mut lines = rest[..end]
                .lines()
                .filter(|line| {
                    line.split_once(':')
                        .is_none_or(|(key, _)| !key.trim().eq_ignore_ascii_case("contexto"))
                })
                .map(str::to_string)
                .collect::<Vec<_>>();
            lines.push(context_line.to_string());
            let body = rest[end + 4..].trim_start_matches('\n');
            return compose(&lines.join("\n"), body);
        }
    }
    compose(context_line, normalized)
}

fn compose(frontmatter: &str, body: &str) -> String {
    if body.is_empty() {
        format!("---\n{frontmatter}\n---\n")
    } else {
        format!("---\n{frontmatter}\n---\n\n{body}")
    }
}

/// Body of an agent document without its frontmatter.
pub fn document_body(source: &str) -> &str {
    strip_frontmatter(source)
}

/// A prompt file name inside `.agent/promps`, or the default prompt.
pub fn normalize_prompt_file_name(file_name: &str) -> String {
    let trimmed = file_name.trim();
    let valid = trimmed.len() > 3
        && trimmed.to_ascii_lowercase().ends_with(".md")
        && !trimmed.contains(['/', '\\'])
        && trimmed != "."
        && trimmed != "..";
    if valid { trimmed.to_string() } else { DEFAULT_PROMPT_FILE.to_string() }
}

/// Prompt file names sorted with the default first.
pub fn prompt_file_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut unique = Vec::<String>::new();
    for name in names.into_iter().map(|name| normalize_prompt_file_name(&name)) {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    if !unique.iter().any(|name| name.eq_ignore_ascii_case(DEFAULT_PROMPT_FILE)) {
        unique.push(DEFAULT_PROMPT_FILE.to_string());
    }
    unique.sort_by(|left, right| {
        let left_default = left.eq_ignore_ascii_case(DEFAULT_PROMPT_FILE);
        let right_default = right.eq_ignore_ascii_case(DEFAULT_PROMPT_FILE);
        right_default.cmp(&left_default).then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
    });
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(append: RuleAppend) -> String {
        match append {
            RuleAppend::Added(body) => body,
            other => panic!("rule not added: {other:?}"),
        }
    }

    #[test]
    fn rules_keep_the_managed_block_and_agent_rules() {
        let first = added(append_rule("", "Cuando pida un resumen, usá viñetas."));
        assert!(first.contains(RULES_START) && first.contains(IA_RULES_START));
        assert_eq!(append_rule(&first, "cuando pida un resumen, usá viñetas."), RuleAppend::Unchanged);
        assert_eq!(ia_rules(&first), vec!["Cuando pida un resumen, usá viñetas.".to_string()]);
        let stale = first.replace(DEFAULT_AGENT_RULES.trim(), &format!("{RULES_START}\nvieja\n{RULES_END}"));
        assert_eq!(ensure_default_rules(&stale), ensure_default_rules(&first));
        let replaced = replace_ia_rules(&first, &["a".into(), "A".into(), " ".into()]);
        assert_eq!(ia_rules(&replaced), vec!["a".to_string()]);
    }

    #[test]
    fn a_full_rules_block_asks_for_a_rewrite_and_rules_have_a_canonical_form() {
        let rules = (0..RULES_LIMIT.items).map(|index| format!("Regla {index}")).collect::<Vec<_>>();
        let full = replace_ia_rules("", &rules);
        assert_eq!(append_rule(&full, "Una más"), RuleAppend::Full);
        assert_eq!(append_rule(&full, "regla 3"), RuleAppend::Unchanged);
        let messy = format!("{}\n\n{IA_RULES_START}\n* uno\n\n- UNO\n-dos\n{IA_RULES_END}", DEFAULT_AGENT_RULES.trim());
        let canonical = canonical_rules(&messy);
        assert_eq!(ia_rules(&canonical), vec!["uno".to_string(), "dos".to_string()]);
        assert_eq!(canonical_rules(&canonical), canonical);
    }

    #[test]
    fn talk_is_a_bounded_list_like_the_memory() {
        let items = vec!["Usa voseo.".to_string(), "usa voseo.".to_string(), "Escribe sin tildes.".to_string()];
        let rendered = TALK.render(&items);
        assert!(rendered.starts_with("<!-- NOTIA_AGENT_TALK_VERSION:1 -->"));
        assert_eq!(parse_memory_items(&rendered), vec!["usa voseo.".to_string(), "Escribe sin tildes.".to_string()]);
        assert!(TALK.target.items < TALK.limit.items && TALK.target.chars < TALK.limit.chars);
        assert!(TALK.max_item_chars <= MAX_RULE_CHARS);
        assert!(RULES_TARGET.items < RULES_LIMIT.items && RULES_TARGET.chars < RULES_LIMIT.chars);
        assert!(BIOGRAPHY_STORY_TARGET < BIOGRAPHY_STORY_LIMIT);
    }

    #[test]
    fn people_are_one_line_per_person() {
        assert_eq!(person_entry("  Ana  ", " su   hermana, vive en Salta. ").as_deref(), Some("Ana — su hermana, vive en Salta."));
        assert_eq!(person_entry("Ana", " "), None);
        assert_eq!(person_entry(" ", "su hermana"), None);
        assert_eq!(person_entry(&"x".repeat(MAX_PERSON_NAME_CHARS + 1), "alguien"), None);
        assert_eq!(person_entry("Ana", &"x".repeat(PEOPLE.max_item_chars)), None);

        assert_eq!(parse_person_entry("- **Ana**: su hermana - vive en Salta").as_deref(), Some("Ana — su hermana - vive en Salta"));
        assert_eq!(parse_person_entry("* Juan Pérez - jefe en Acme: le reporta").as_deref(), Some("Juan Pérez — jefe en Acme: le reporta"));
        assert_eq!(parse_person_entry("- Ana — su hermana").as_deref(), Some("Ana — su hermana"));
        assert_eq!(parse_person_entry("Ana"), None);

        let rendered = PEOPLE.render(&["Ana — su hermana".into(), "ana — su hermana".into()]);
        assert!(rendered.starts_with("<!-- NOTIA_AGENT_PEOPLE_VERSION:1 -->"));
        assert_eq!(parse_memory_items(&rendered), vec!["ana — su hermana".to_string()]);
        assert!(PEOPLE.target.items < PEOPLE.limit.items && PEOPLE.target.chars < PEOPLE.limit.chars);
        assert!(PEOPLE.max_item_chars <= MAX_RULE_CHARS);
    }

    #[test]
    fn the_biography_is_a_story_with_facts_waiting_to_be_told() {
        let empty = Biography::default();
        assert_eq!(empty.render(), format!("{BIOGRAPHY_MARKER}\n"));
        assert_eq!(Biography::parse(&empty.render()), empty);
        assert!(!empty.needs_writing());

        let BiographyAppend::Added(noted) = empty.with_note("  Nació en   Salta. ") else { panic!("added") };
        assert_eq!(noted.notes, vec!["Nació en Salta.".to_string()]);
        assert!(noted.needs_writing());
        assert_eq!(noted.with_note("nació en salta."), BiographyAppend::Unchanged);
        assert_eq!(noted.with_note(" "), BiographyAppend::Unchanged);

        let told = Biography {
            story: "# La vida de Ana\n\n## Origen\n\n\n\nAna nació en Salta, en 1990.".into(),
            notes: vec!["Estudió en la UNSa.".into()],
        };
        let rendered = told.render();
        assert!(rendered.contains("## Origen\n\nAna nació") && rendered.contains("### Datos por incorporar\n\n- Estudió en la UNSa."));
        let parsed = Biography::parse(&rendered);
        assert_eq!(parsed.notes, told.notes);
        assert_eq!(parsed.story, "# La vida de Ana\n\n## Origen\n\nAna nació en Salta, en 1990.");
        assert_eq!(Biography::parse(&parsed.render()), parsed);

        // A list from the first version becomes facts to tell.
        let old = format!("{BIOGRAPHY_LIST_MARKER}\n\n- Nació en Salta.\n- Estudió en la UNSa.\n");
        assert_eq!(Biography::parse(&old), Biography { story: String::new(), notes: vec!["Nació en Salta.".into(), "Estudió en la UNSa.".into()] });
        // A hand-written text without markers is the story.
        assert_eq!(Biography::parse("Ana nació en Salta.").story, "Ana nació en Salta.");

        let full = Biography { story: String::new(), notes: (0..BIOGRAPHY_NOTES_LIMIT.items).map(|index| format!("Dato {index}")).collect() };
        assert_eq!(full.with_note("Uno más"), BiographyAppend::Full);
        assert!(Biography { story: "x".repeat(BIOGRAPHY_STORY_TARGET + 1), notes: Vec::new() }.needs_writing());
    }

    #[test]
    fn personal_facts_move_to_memory_and_corrections_are_dropped() {
        let content = replace_ia_rules(
            "",
            &[
                "El usuario se llama Ana".into(),
                "Ninguna mutación financiera se ejecutó".into(),
                "Respondé en inglés".into(),
                "Tienda: no es memoria".into(),
            ],
        );
        let (rules, memories) = migrate_misclassified_rules(&content);
        assert_eq!(memories, vec!["El usuario se llama Ana".to_string()]);
        assert_eq!(ia_rules(&rules), vec!["Respondé en inglés".to_string(), "Tienda: no es memoria".to_string()]);
    }

    #[test]
    fn memories_are_parsed_merged_and_rendered() {
        let items = parse_memory_items("<!-- marker -->\n# Título\n- uno\n* Dos\n\n");
        assert_eq!(items, vec!["uno".to_string(), "Dos".to_string()]);
        let merged = merge_memories(items.into_iter().chain(["UNO".to_string()]));
        assert_eq!(merged, vec!["UNO".to_string(), "Dos".to_string()]);
        assert_eq!(parse_memory_items(&render_memories(&merged)), merged);
    }

    #[test]
    fn thoughts_are_dated_refreshed_and_rendered() {
        let first = stamp_thought("2026-09-27 10:00", "  Le avisé del turno   del lunes. ").expect("thought");
        assert_eq!(first, "[2026-09-27 10:00] Le avisé del turno del lunes.");
        assert_eq!(thought_text(&first), "Le avisé del turno del lunes.");
        assert_eq!(thought_text("sin fecha"), "sin fecha");
        assert_eq!(stamp_thought("2026-09-27 10:00", " "), None);
        assert_eq!(stamp_thought("2026-09-27 10:00", &"x".repeat(MAX_THOUGHT_CHARS + 1)), None);

        let other = stamp_thought("2026-09-27 10:05", "Propuse ordenar la rutina.").expect("thought");
        let again = stamp_thought("2026-09-27 11:00", "le avisé del turno del lunes.").expect("thought");
        let thoughts = with_thought(&with_thought(&[first], other.clone()), again.clone());
        assert_eq!(thoughts, vec![other.clone(), again.clone()]);
        // Almost the same words is the same note; another subject is not.
        let reworded = stamp_thought("2026-09-27 12:00", "Le avisé del turno de los lunes").expect("thought");
        assert_eq!(with_thought(&thoughts, reworded.clone()), vec![other.clone(), reworded]);
        let unrelated = stamp_thought("2026-09-27 12:00", "Le avisé del turno del dentista el martes.").expect("thought");
        assert_eq!(with_thought(&thoughts, unrelated).len(), 3);

        let rendered = render_thoughts(&thoughts);
        assert!(rendered.starts_with(THOUGHTS_VERSION_MARKER));
        assert_eq!(parse_memory_items(&rendered), thoughts);

        // A full file loses its oldest notes, never the new one.
        let full = vec!["a".repeat(40), "b".repeat(40), "c".repeat(40), "d".repeat(10)];
        let (kept, dropped) = without_oldest(full.clone(), ItemBudget { items: 10, chars: 60 });
        assert_eq!((kept, dropped), (vec!["c".repeat(40), "d".repeat(10)], 2));
        assert_eq!(without_oldest(full.clone(), ItemBudget { items: 2, chars: 1_000 }).1, 2);
        assert_eq!(without_oldest(full.clone(), ItemBudget { items: 10, chars: 1_000 }), (full, 0));
        let (kept, _) = without_oldest(vec!["x".repeat(100)], ItemBudget { items: 10, chars: 5 });
        assert_eq!(kept.len(), 1, "the last note stays even when it alone passes the size");
    }

    #[test]
    fn budgets_count_items_and_characters() {
        let items = vec!["abc".to_string(), "de".to_string()];
        let budget = ItemBudget { items: 3, chars: 6 };
        assert!(budget.fits(&items));
        assert!(budget.fits_one_more(&items, "f"));
        assert!(!budget.fits_one_more(&items, "fg"));
        assert!(!ItemBudget { items: 2, chars: 100 }.fits_one_more(&items, "f"));
        assert!(!ItemBudget { items: 5, chars: 4 }.fits(&items));
        assert!(MEMORY_TARGET.items < MEMORY_LIMIT.items && MEMORY_TARGET.chars < MEMORY_LIMIT.chars);
        assert!(THOUGHTS_TARGET.items < THOUGHTS_LIMIT.items && THOUGHTS_TARGET.chars < THOUGHTS_LIMIT.chars);
    }

    #[test]
    fn confidential_context_replaces_the_context_and_keeps_the_body() {
        assert_eq!(with_confidential_context("hola"), "---\ncontexto: \"#Confidencial\"\n---\n\nhola");
        let marked = with_confidential_context("---\ntitle: x\ncontexto: \"#Personal\"\n---\n\nhola");
        assert_eq!(marked, "---\ntitle: x\ncontexto: \"#Confidencial\"\n---\n\nhola");
        assert_eq!(with_confidential_context(&marked), marked);
    }

    #[test]
    fn the_files_only_the_ai_writes_are_system_files() {
        for path in SYSTEM_FILES {
            assert!(is_system_file(path) && holds_system_files(path), "{path}");
        }
        assert!(is_system_file(" .agent/Memory/memory.md "));
        assert!(!is_system_file(".agent/promps/default.md") && !is_system_file(".agent/skills/x.md") && !is_system_file("memory.md"));
        assert!(holds_system_files(".agent") && holds_system_files(".agent/memory/") && holds_system_files(".AGENT/memory"));
        assert!(!holds_system_files(".agent/promps") && !holds_system_files(".agen") && !holds_system_files(""));
        // The scripts' guide is Notia's; the scripts next to it are not.
        assert!(is_system_file(".agent/scripts/README.md") && holds_system_files(".agent/scripts"));
        assert!(!is_system_file(".agent/scripts/csv_a_tabla.py") && !holds_system_files(".agent/scripts/csv_a_tabla.py"));

        let memory = with_agent_context(MEMORY_PATH, "---\ncontexto: \"#Confidencial\"\n---\n\n- Usa voseo.");
        assert_eq!(memory, "---\ncontexto: \"#System\"\n---\n\n- Usa voseo.");
        assert_eq!(with_agent_context(".agent/skills/x.md", "hola"), "---\ncontexto: \"#Confidencial\"\n---\n\nhola");
    }

    #[test]
    fn prompt_names_are_validated_and_sorted() {
        assert_eq!(normalize_prompt_file_name("../x.md"), DEFAULT_PROMPT_FILE);
        assert_eq!(
            prompt_file_names(["Zeta.md".into(), "alfa.md".into(), "notas.txt".into()]),
            vec!["default.md".to_string(), "alfa.md".to_string(), "Zeta.md".to_string()]
        );
    }

    #[test]
    fn improvements_are_dated_typed_unique_and_bounded() {
        let entry = |text: &str| improvement_entry("2026-10-25 21:40", ImprovementKind::parse("herramienta"), text).expect("entry");
        let first = entry("Falta  una herramienta para
renombrar carpetas.");
        assert_eq!(first, "2026-10-25 21:40 · herramienta · Falta una herramienta para renombrar carpetas.");
        assert_eq!(ImprovementKind::parse("Error"), ImprovementKind::Error);
        assert_eq!(ImprovementKind::parse("cualquier cosa"), ImprovementKind::Improvement);
        assert!(improvement_entry("x", ImprovementKind::Error, " ").is_none());
        assert!(improvement_entry("x", ImprovementKind::Error, &"a".repeat(MAX_IMPROVEMENT_CHARS + 1)).is_none());

        let entries = with_improvement(&[], first.clone()).expect("added");
        // The same improvement another day is not repeated.
        let again = improvement_entry("2026-10-26 09:00", ImprovementKind::Tool, "falta una herramienta para renombrar carpetas.").expect("entry");
        assert_eq!(with_improvement(&entries, again), None);
        // The same text of another kind is another entry.
        let error = improvement_entry("2026-10-26 09:00", ImprovementKind::Error, "Falta una herramienta para renombrar carpetas.").expect("entry");
        assert_eq!(with_improvement(&entries, error).map(|next| next.len()), Some(2));

        let body = render_improvements(&entries);
        assert!(body.starts_with(IMPROVE_MARKER) && body.contains("# Mejoras para Notia"));
        assert_eq!(parse_improvements(&body), entries);

        let full = (0..IMPROVE_LIMIT.items).map(|index| format!("2026-01-01 00:00 · mejora · punto {index}")).collect::<Vec<_>>();
        let next = with_improvement(&full, entry("Una nueva.")).expect("added");
        assert_eq!(next.len(), IMPROVE_LIMIT.items);
        assert_eq!(next.first().map(String::as_str), Some("2026-01-01 00:00 · mejora · punto 1"));
        assert!(next.last().is_some_and(|last| last.ends_with("Una nueva.")));
    }
}
