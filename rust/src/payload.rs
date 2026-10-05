//! Payload parser for toast / tile / badge XML.
//!
//! Mirrors `winnotifylog/toast_parser.py`: the same fields are extracted with the
//! same defensive behaviour.  A payload that is not well-formed XML (Windows
//! stores truncated or condensed payloads) falls back to a tag scan instead of
//! failing - the original text is always stored in the journal untouched.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde_json::{json, Value as Json};

/// Fields extracted from a notification payload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedPayload {
    pub title: Option<String>,
    pub body: Option<String>,
    pub extra_text: Vec<String>,
    pub images: Vec<String>,
    pub actions: Vec<Json>,
    pub launch: Option<String>,
    pub activation_type: Option<String>,
    pub scenario: Option<String>,
    pub duration: Option<String>,
    pub audio: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub data: BTreeMap<String, String>,
    /// True when the payload was well-formed XML.
    pub parsed: bool,
}

impl ParsedPayload {
    /// Title, body and the remaining text lines, in order.
    pub fn texts(&self) -> Vec<String> {
        let mut values: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            values.push(title.clone());
        }
        if let Some(body) = &self.body {
            values.push(body.clone());
        }
        values.extend(self.extra_text.iter().cloned());
        values
    }
}

/// Parse a payload; never fails.
pub fn parse(payload: Option<&str>) -> ParsedPayload {
    let text = match payload {
        Some(value) => value.trim_start_matches('\u{feff}'),
        None => return ParsedPayload::default(),
    };
    if text.trim().is_empty() {
        return ParsedPayload::default();
    }
    match parse_xml(text) {
        Ok(parsed) if parsed.parsed => {
            // quick-xml tolerates truncated payloads: it stops at the end of the
            // document without complaining.  When fewer text elements were
            // collected than the payload contains, the tag scan is more complete.
            let expected = count_occurrences(text, "<text");
            if parsed.texts().len() < expected {
                let recovered = fallback(text);
                if recovered.texts().len() > parsed.texts().len() {
                    return recovered;
                }
            }
            parsed
        }
        // No elements at all: the value is not XML, so treat it as opaque text.
        Ok(_) => fallback(text),
        Err(_) => fallback(text),
    }
}

/// How often `needle` occurs in `text` (case-insensitive).
fn count_occurrences(text: &str, needle: &str) -> usize {
    text.to_lowercase().matches(needle).count()
}

fn parse_xml(text: &str) -> Result<ParsedPayload, quick_xml::Error> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut result = ParsedPayload::default();
    let mut saw_element = false;
    let mut texts: Vec<String> = Vec::new();
    let mut buffer = String::new();
    let mut current = String::new();
    let mut pending_action: Option<Json> = None;

    loop {
        match reader.read_event()? {
            Event::Start(event) => {
                saw_element = true;
                handle_start(
                    &event,
                    &mut result,
                    &mut current,
                    &mut buffer,
                    &mut pending_action,
                );
            }
            Event::Empty(event) => {
                saw_element = true;
                handle_start(
                    &event,
                    &mut result,
                    &mut current,
                    &mut buffer,
                    &mut pending_action,
                );
                let name = local_name_of(&event);
                handle_end(
                    &name,
                    &mut result,
                    &mut texts,
                    &mut buffer,
                    &mut pending_action,
                );
            }
            Event::Text(text) => {
                let value = text.unescape().unwrap_or_default().into_owned();
                buffer.push_str(&value);
            }
            Event::CData(text) => {
                buffer.push_str(&String::from_utf8_lossy(text.as_ref()));
            }
            Event::End(event) => {
                let name =
                    String::from_utf8_lossy(event.local_name().as_ref()).to_ascii_lowercase();
                handle_end(
                    &name,
                    &mut result,
                    &mut texts,
                    &mut buffer,
                    &mut pending_action,
                );
            }
            Event::Eof => break,
            _ => {}
        }
    }

    assign_texts(&mut result, texts);
    result.parsed = saw_element;
    Ok(result)
}

fn handle_start(
    event: &BytesStart<'_>,
    result: &mut ParsedPayload,
    current: &mut String,
    buffer: &mut String,
    pending_action: &mut Option<Json>,
) {
    let name = String::from_utf8_lossy(event.local_name().as_ref()).to_ascii_lowercase();
    *current = name.clone();
    buffer.clear();

    let mut attributes: BTreeMap<String, String> = BTreeMap::new();
    for attribute in event.attributes().flatten() {
        let key = String::from_utf8_lossy(attribute.key.as_ref()).to_lowercase();
        let value = attribute
            .unescape_value()
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attribute.value).into_owned());
        attributes.insert(key, value);
    }

    match name.as_str() {
        "toast" => {
            result.attributes = attributes.clone();
            result.launch = attributes.get("launch").cloned();
            result.activation_type = attributes
                .get("activationtype")
                .cloned()
                .or_else(|| attributes.get("activation_type").cloned());
            result.scenario = attributes.get("scenario").cloned();
            result.duration = attributes.get("duration").cloned();
        }
        "image" => {
            // `<image src=""/>` does happen in real payloads; an empty
            // reference is not a picture.
            if let Some(source) = attributes
                .get("src")
                .or_else(|| attributes.get("source"))
                .filter(|value| !value.trim().is_empty())
            {
                result.images.push(source.clone());
            }
        }
        "audio" => {
            if let Some(source) = attributes.get("src") {
                result.audio = Some(source.clone());
            }
        }
        "action" => {
            *pending_action = Some(json!(attributes));
        }
        "data" => {
            *pending_action = Some(json!(attributes));
        }
        _ => {}
    }
}

fn handle_end(
    name: &str,
    result: &mut ParsedPayload,
    texts: &mut Vec<String>,
    buffer: &mut String,
    pending_action: &mut Option<Json>,
) {
    match name {
        "text" => {
            let value = buffer.trim();
            if !value.is_empty() {
                texts.push(value.to_string());
            }
        }
        "action" => {
            if let Some(Json::Object(mut map)) = pending_action.take() {
                let text = buffer.trim();
                if !text.is_empty() && !map.contains_key("content") {
                    map.insert("content".to_string(), Json::String(text.to_string()));
                }
                result.actions.push(Json::Object(map));
            }
        }
        "data" => {
            if let Some(Json::Object(map)) = pending_action.take() {
                let key = map
                    .get("id")
                    .or_else(|| map.get("key"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                if let Some(key) = key {
                    result.data.insert(key, buffer.trim().to_string());
                }
            }
        }
        _ => {}
    }
    buffer.clear();
}

fn assign_texts(result: &mut ParsedPayload, texts: Vec<String>) {
    let mut iterator = texts.into_iter();
    result.title = iterator.next();
    result.body = iterator.next();
    result.extra_text = iterator.collect();
}

/// Lowercased local name of an element (`<uap:text>` becomes `text`).
fn local_name_of(event: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(event.local_name().as_ref()).to_ascii_lowercase()
}

// ------------------------------------------------------------ fallback scanning
/// Best effort extraction from a payload that is not valid XML.
fn fallback(text: &str) -> ParsedPayload {
    let mut result = ParsedPayload {
        parsed: false,
        ..Default::default()
    };
    assign_texts(&mut result, collect_texts(text));

    for attributes in scan_attributes(text, "<image") {
        if let Some(source) = attributes
            .get("src")
            .or_else(|| attributes.get("source"))
            .filter(|value| !value.trim().is_empty())
        {
            result.images.push(source.clone());
        }
    }
    for attributes in scan_attributes(text, "<action") {
        let mut map = serde_json::Map::new();
        for (key, value) in attributes {
            map.insert(key, Json::String(value));
        }
        if !map.is_empty() {
            result.actions.push(Json::Object(map));
        }
    }
    for attributes in scan_attributes(text, "<toast") {
        for (key, value) in &attributes {
            result.attributes.insert(key.clone(), value.clone());
        }
        if let Some(value) = attributes.get("launch") {
            result.launch = Some(value.clone());
        }
        if let Some(value) = attributes.get("activationtype") {
            result.activation_type = Some(value.clone());
        }
    }
    result
}

/// Collect the text of every `<text>` element, including truncated (unclosed) ones.
fn collect_texts(text: &str) -> Vec<String> {
    const CLOSE: &str = "</text>";
    let lowered = text.to_lowercase();
    let mut values = Vec::new();
    let mut offset = 0;
    while let Some(found) = lowered[offset..].find("<text") {
        let start = offset + found;
        let Some(open_end) = lowered[start..].find('>') else {
            break;
        };
        let content_start = start + open_end + 1;
        if let Some(close_at) = lowered[content_start..].find(CLOSE) {
            let raw = &text[content_start..content_start + close_at];
            let cleaned = strip_tags(raw).trim().to_string();
            if !cleaned.is_empty() {
                values.push(cleaned);
            }
            offset = content_start + close_at + CLOSE.len();
        } else {
            // Truncated payload: keep what follows the unclosed element.
            let remaining = &text[content_start..];
            let end = remaining.find('<').unwrap_or(remaining.len());
            let value = remaining[..end].trim();
            if !value.is_empty() {
                values.push(value.to_string());
            }
            offset = content_start + end.max(1);
        }
    }
    values
}

/// Remove nested tags from a fragment and keep its text.
fn strip_tags(fragment: &str) -> String {
    let mut output = String::with_capacity(fragment.len());
    let mut inside = false;
    for character in fragment.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            other if !inside => output.push(other),
            _ => {}
        }
    }
    output
}

/// Read the attributes of every `<prefix ...>` occurrence (case-insensitive).
fn scan_attributes(text: &str, prefix: &str) -> Vec<BTreeMap<String, String>> {
    let lowered = text.to_lowercase();
    let mut results = Vec::new();
    let mut offset = 0;
    while let Some(start) = lowered[offset..].find(prefix) {
        let start = offset + start;
        let Some(open_end) = lowered[start..].find('>') else {
            break;
        };
        let raw = &text[start + prefix.len()..start + open_end];
        let mut attributes = BTreeMap::new();
        let mut characters = raw.chars().peekable();
        let mut key = String::new();
        let mut value: Option<String> = None;
        let mut quote: Option<char> = None;
        for character in characters.by_ref() {
            match (quote, character) {
                (Some(active), current) if current == active => {
                    if let Some(existing) = value.take() {
                        if !key.trim().is_empty() {
                            attributes.insert(key.trim().to_lowercase(), existing);
                        }
                    }
                    key.clear();
                    quote = None;
                }
                (Some(_), current) => {
                    if let Some(existing) = value.as_mut() {
                        existing.push(current);
                    }
                }
                (None, '"') | (None, '\'') => {
                    quote = Some(character);
                    value = Some(String::new());
                }
                (None, '=') => {}
                (None, current) => {
                    key.push(current);
                }
            }
        }
        if !attributes.is_empty() {
            results.push(attributes);
        }
        offset = start + open_end;
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOAST: &str = r#"<toast launch="app:action" activationType="foreground" scenario="reminder">
  <visual>
    <binding template="ToastGeneric">
      <text>Alice</text>
      <text>See you at 19:00</text>
      <text>Second line</text>
      <image src="file:///C:/avatar.png" placement="appLogoOverride"/>
    </binding>
  </visual>
  <actions>
    <action content="Reply" arguments="reply" activationType="background"/>
    <action content="Dismiss" arguments="dismiss" activationType="system"/>
  </actions>
  <audio src="ms-winsoundevent:Notification.Reminder"/>
</toast>"#;

    #[test]
    fn parses_a_toast_generic_payload() {
        let parsed = parse(Some(TOAST));
        assert!(parsed.parsed);
        assert_eq!(parsed.title.as_deref(), Some("Alice"));
        assert_eq!(parsed.body.as_deref(), Some("See you at 19:00"));
        assert_eq!(parsed.extra_text, vec!["Second line".to_string()]);
        assert_eq!(parsed.images, vec!["file:///C:/avatar.png".to_string()]);
        assert_eq!(parsed.actions.len(), 2);
        assert_eq!(
            parsed.actions[0]
                .get("content")
                .and_then(|value| value.as_str()),
            Some("Reply")
        );
        assert_eq!(parsed.launch.as_deref(), Some("app:action"));
        assert_eq!(parsed.activation_type.as_deref(), Some("foreground"));
        assert_eq!(parsed.scenario.as_deref(), Some("reminder"));
        assert_eq!(
            parsed.audio.as_deref(),
            Some("ms-winsoundevent:Notification.Reminder")
        );
    }

    /// Real payloads do contain `<image src=""/>`; such a reference is not a
    /// picture and must not turn into an empty entry in the journal.
    #[test]
    fn empty_picture_references_are_not_recorded() {
        let parsed = parse(Some(
            r#"<toast><visual><binding template="ToastGeneric"><text>Title</text>
            <image id="1" src="" placement="Background"/>
            <image src="ms-appx:///Assets/LiveTiles/avatar150x150.png" placement="peek"/>
            </binding></visual></toast>"#,
        ));
        assert_eq!(
            parsed.images,
            vec!["ms-appx:///Assets/LiveTiles/avatar150x150.png".to_string()]
        );
    }

    #[test]
    fn parses_a_tile_payload() {
        let payload = r#"<tile><visual><binding template="TileWide">
            <text id="1">Tile title</text><text id="2">Tile body</text>
        </binding></visual></tile>"#;
        let parsed = parse(Some(payload));
        assert_eq!(parsed.title.as_deref(), Some("Tile title"));
        assert_eq!(parsed.body.as_deref(), Some("Tile body"));
    }

    #[test]
    fn parses_data_bindings() {
        let payload = r#"<toast><visual><binding template="ToastGeneric"><text>T</text></binding></visual>
            <binding template="ToastGeneric"><data id="url">https://example.com</data></binding></toast>"#;
        let parsed = parse(Some(payload));
        assert_eq!(
            parsed.data.get("url").map(String::as_str),
            Some("https://example.com")
        );
    }

    #[test]
    fn keeps_cyrillic_text() {
        let payload = "<toast><visual><binding template=\"ToastGeneric\"><text>Привет</text><text>Мир</text></binding></visual></toast>";
        let parsed = parse(Some(payload));
        assert_eq!(parsed.title.as_deref(), Some("Привет"));
        assert_eq!(parsed.body.as_deref(), Some("Мир"));
    }

    #[test]
    fn junk_payloads_are_ignored() {
        for payload in [None, Some(""), Some("   ")] {
            let parsed = parse(payload);
            assert!(!parsed.parsed);
            assert!(parsed.texts().is_empty());
        }
        // Bytes that are not XML at all are not an error either: the record is
        // still stored, it simply carries no text.
        let rubbish = parse(Some("\u{1}\u{2} not xml at all \u{3}"));
        assert!(!rubbish.parsed);
        assert!(rubbish.texts().is_empty());
    }

    #[test]
    fn recovers_text_from_truncated_payloads() {
        // The notification database keeps a payload that was cut off in the middle.
        let parsed = parse(Some(
            "<toast><visual><binding template=\"ToastGeneric\"><text>Broken",
        ));
        assert!(!parsed.parsed);
        assert_eq!(parsed.title.as_deref(), Some("Broken"));

        // Attributes that arrived before the cut are kept as well.
        let parsed = parse(Some(
            "<toast launch=\"app:test\"><text>Title</text><text>Body",
        ));
        assert!(!parsed.parsed);
        assert_eq!(parsed.title.as_deref(), Some("Title"));
        assert_eq!(parsed.body.as_deref(), Some("Body"));
        assert_eq!(parsed.launch.as_deref(), Some("app:test"));
    }

    #[test]
    fn recognises_namespaced_elements() {
        let payload = r#"<toast xmlns="http://schemas.microsoft.com/notification/toast"><visual>
            <binding template="ToastGeneric"><text>Ns title</text></binding></visual></toast>"#;
        assert_eq!(parse(Some(payload)).title.as_deref(), Some("Ns title"));
    }

    #[test]
    fn unescapes_entities() {
        let payload = "<toast><visual><binding template=\"ToastGeneric\"><text>A &amp; B</text></binding></visual></toast>";
        assert_eq!(parse(Some(payload)).title.as_deref(), Some("A & B"));
    }

    #[test]
    fn texts_helper_keeps_the_order() {
        let parsed = parse(Some(TOAST));
        assert_eq!(
            parsed.texts(),
            vec![
                "Alice".to_string(),
                "See you at 19:00".to_string(),
                "Second line".to_string()
            ]
        );
    }
}
