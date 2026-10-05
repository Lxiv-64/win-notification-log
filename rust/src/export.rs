//! Exporting the journal to a file the user chooses.
//!
//! Four formats are offered, because the right one depends on what happens next:
//! CSV for a spreadsheet, JSON for a script, TXT for reading and HTML for
//! printing or sharing.  The dialog is the standard Windows "save as" dialog, so
//! the folder, the name and the format are chosen in one place.

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use crate::sqlite::Row;
use crate::view;
use crate::win32::{self, HWND};

/// Selectable output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Txt,
    Html,
}

impl Format {
    /// Every format, in the order the dialog lists them.
    pub const ALL: [Format; 4] = [Format::Csv, Format::Json, Format::Txt, Format::Html];

    pub fn name(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Txt => "txt",
            Format::Html => "html",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => tr!("Таблица CSV (Excel)"),
            Format::Json => tr!("Данные JSON"),
            Format::Txt => tr!("Текст TXT"),
            Format::Html => tr!("Страница HTML"),
        }
    }

    pub fn extension(self) -> &'static str {
        self.name()
    }

    pub fn pattern(self) -> &'static str {
        match self {
            Format::Csv => "*.csv",
            Format::Json => "*.json",
            Format::Txt => "*.txt",
            Format::Html => "*.html",
        }
    }

    /// Format of a configured name; unknown names become CSV.
    pub fn from_name(name: &str) -> Format {
        match name.trim().to_ascii_lowercase().as_str() {
            "json" => Format::Json,
            "txt" | "text" => Format::Txt,
            "html" | "htm" => Format::Html,
            _ => Format::Csv,
        }
    }

    /// Format implied by a file name (used when the user types an extension).
    pub fn from_path(path: &Path) -> Option<Format> {
        let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
        Format::ALL.into_iter().find(|format| {
            format.extension() == extension || (format == &Format::Html && extension == "htm")
        })
    }
}

/// Filter list for the "save as" dialog: every format plus "all files".
pub fn filters() -> Vec<(String, String)> {
    let mut filters: Vec<(String, String)> = Format::ALL
        .into_iter()
        .map(|format| (format.label().to_string(), format.pattern().to_string()))
        .collect();
    filters.push((tr!("Все файлы (*.*)").to_string(), "*.*".to_string()));
    filters
}

/// File name suggested by the dialog (`notifications-20260929-214735.csv`).
pub fn suggested_name(format: Format, stamp: &str) -> String {
    format!("notifications-{stamp}.{}", format.extension())
}

/// Quote a CSV field when it contains the separator, a quote or a newline.
pub fn csv_field(value: &str) -> String {
    if value.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Escape text for HTML.
pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// All exported fields of one row (the order is the CSV column order).
fn fields(row: &Row) -> Vec<(&'static str, String)> {
    vec![
        ("id", row.int("id").unwrap_or_default().to_string()),
        (
            "first_seen_utc",
            row.text("first_seen_utc").unwrap_or_default(),
        ),
        (
            "local_time",
            view::local_time(row.text("first_seen_utc").as_deref()),
        ),
        ("app", view::display_app(row)),
        ("aumid", row.optional_text("aumid").unwrap_or_default()),
        (
            "notification_type",
            row.text("notification_type").unwrap_or_default(),
        ),
        (
            "title",
            view::single_line(row.optional_text("title").unwrap_or_default().as_str()),
        ),
        (
            "body",
            view::single_line(row.optional_text("body").unwrap_or_default().as_str()),
        ),
        ("actions", view::actions(row).join(" | ")),
        ("images", view::images(row).join(" | ")),
        ("launch", row.optional_text("launch").unwrap_or_default()),
        ("sources", row.text("sources").unwrap_or_default()),
        (
            "seen_count",
            row.int("seen_count").unwrap_or_default().to_string(),
        ),
        (
            "revision",
            row.int("revision").unwrap_or_default().to_string(),
        ),
        ("dedup_key", row.text("dedup_key").unwrap_or_default()),
    ]
}

/// Render rows in the requested format.
pub fn render(rows: &[Row], format: Format, stamp: &str) -> String {
    match format {
        Format::Csv => render_csv(rows),
        Format::Json => render_json(rows, stamp),
        Format::Txt => render_txt(rows, stamp),
        Format::Html => render_html(rows, stamp),
    }
}

fn render_csv(rows: &[Row]) -> String {
    let mut text = String::from(
        "id;utc;local;app;aumid;type;title;body;actions;images;launch;sources;seen;revision;key\r\n",
    );
    for row in rows {
        let line: Vec<String> = fields(row)
            .into_iter()
            .map(|(_, value)| csv_field(&value))
            .collect();
        text.push_str(&line.join(";"));
        text.push_str("\r\n");
    }
    // The byte order mark makes Excel read the file as UTF-8.
    format!("\u{feff}{text}")
}

fn render_json(rows: &[Row], stamp: &str) -> String {
    let notifications: Vec<Json> = rows
        .iter()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for (key, value) in fields(row) {
                object.insert(key.to_string(), Json::String(value));
            }
            object.insert(
                "seen_count".to_string(),
                json!(row.int("seen_count").unwrap_or(1)),
            );
            object.insert(
                "revision".to_string(),
                json!(row.int("revision").unwrap_or(1)),
            );
            Json::Object(object)
        })
        .collect();
    let document = json!({
        "application": tr!("Журнал уведомлений Windows"),
        "exported_local": stamp,
        "count": rows.len(),
        "notifications": notifications,
    });
    serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_string())
}

fn render_txt(rows: &[Row], stamp: &str) -> String {
    let mut text = tf!(
        "Журнал уведомлений Windows\r\nВыгрузка: {stamp}\r\nЗаписей: {}\r\n",
        "Windows notification journal\r\nExport: {stamp}\r\nRecords: {}\r\n",
        rows.len()
    );
    let separator = "\r\n".to_string() + &"-".repeat(70) + "\r\n";
    for row in rows {
        text.push_str(&separator);
        text.push_str(&format!(
            "{}   {}\r\n",
            view::local_time(row.text("first_seen_utc").as_deref()),
            view::display_app(row)
        ));
        for (label, value) in view::readable_lines(row) {
            if label == tr!("Время") || label == tr!("Приложение") {
                continue;
            }
            text.push_str(&format!("{label}: {value}\r\n"));
        }
    }
    text
}

fn render_html(rows: &[Row], stamp: &str) -> String {
    let mut text = String::from(
        "<!DOCTYPE html>\n<html lang=\"ru\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Журнал уведомлений Windows</title>\n<style>\n\
         body { font-family: 'Segoe UI', sans-serif; background: #f6f7f9; color: #1c1c1c; \
         margin: 0; padding: 24px; }\n\
         h1 { font-size: 20px; margin: 0 0 4px; }\n\
         .meta { color: #5c5c5c; margin-bottom: 18px; }\n\
         .card { background: #fff; border: 1px solid #e2e4e8; border-radius: 8px; \
         padding: 14px 16px; margin-bottom: 12px; }\n\
         .card h2 { font-size: 15px; margin: 0 0 2px; }\n\
         .card .app { color: #4a6fa5; font-size: 13px; margin-bottom: 8px; }\n\
         .card .body { white-space: pre-wrap; }\n\
         dl { margin: 8px 0 0; font-size: 12px; color: #5c5c5c; }\n\
         dt { font-weight: 600; }\n\
         dd { margin: 0 0 4px 0; white-space: pre-wrap; }\n\
         </style>\n</head>\n<body>\n",
    );
    // The title and the language marker of the page follow the interface
    // language; the markup itself is language-neutral.
    let page_lang = format!("lang=\"{}\"", crate::lang::current().code());
    text = text
        .replace("lang=\"ru\"", &page_lang)
        .replace(
            "Журнал уведомлений Windows",
            tr!("Журнал уведомлений Windows"),
        );
    text.push_str(&tf!(
        "<h1>Журнал уведомлений Windows</h1>\n<div class=\"meta\">Выгрузка: {} · записей: {}</div>\n",
        "<h1>Windows notification journal</h1>\n<div class=\"meta\">Export: {} · records: {}</div>\n",
        escape_html(stamp),
        rows.len()
    ));
    for row in rows {
        let title = row.optional_text("title").unwrap_or_default();
        let heading = if title.is_empty() {
            tr!("(без заголовка)")
        } else {
            title.as_str()
        };
        text.push_str("<div class=\"card\">\n");
        text.push_str(&format!(
            "<h2>{}</h2>\n<div class=\"app\">{} · {}</div>\n",
            escape_html(heading),
            escape_html(&view::display_app(row)),
            escape_html(&view::local_time(row.text("first_seen_utc").as_deref()))
        ));
        let body = row.optional_text("body").unwrap_or_default();
        if !body.trim().is_empty() {
            text.push_str(&format!(
                "<div class=\"body\">{}</div>\n",
                escape_html(&body)
            ));
        }
        let technical = view::technical_lines(row);
        if !technical.is_empty() {
            text.push_str("<dl>\n");
            for (label, value) in technical {
                text.push_str(&format!(
                    "<dt>{}</dt><dd>{}</dd>\n",
                    escape_html(&label),
                    escape_html(&value)
                ));
            }
            text.push_str("</dl>\n");
        }
        text.push_str("</div>\n");
    }
    text.push_str("</body>\n</html>\n");
    text
}

/// Write the rows where the user asked for them.
///
/// The extension of the chosen name wins over the menu entry, so typing
/// `report.json` while "CSV" is selected still produces JSON.
pub fn save(
    owner: HWND,
    rows: &[Row],
    format: Format,
    suggested: &str,
    stamp: &str,
) -> Result<PathBuf, String> {
    let filters = filters();
    let borrowed: Vec<(&str, &str)> = filters
        .iter()
        .map(|(caption, pattern)| (caption.as_str(), pattern.as_str()))
        .collect();
    let path = win32::save_file_name(
        owner,
        "Экспорт журнала уведомлений",
        &borrowed,
        format.extension(),
        suggested,
    )
    .ok_or_else(|| tr!("Экспорт отменён.").to_string())?;
    let chosen = Format::from_path(&path).unwrap_or(format);
    let text = render(rows, chosen, stamp);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, text)
        .map_err(|error| tf!("Не удалось записать файл: {error}", "Could not write the file: {error}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::Value;

    fn row(pairs: &[(&str, &str)]) -> Row {
        let names: Vec<String> = pairs.iter().map(|(key, _)| key.to_string()).collect();
        let values: Vec<Value> = pairs
            .iter()
            .map(|(_, value)| Value::Text(value.to_string()))
            .collect();
        Row::from_pairs(names, values)
    }

    fn sample() -> Vec<Row> {
        vec![
            row(&[
                ("id", "1"),
                ("first_seen_utc", "2026-09-29T10:11:12.000000Z"),
                ("app_name", "Почта"),
                ("title", "Новое письмо"),
                ("body", "Привет; \"кавычки\""),
                ("seen_count", "2"),
                ("revision", "1"),
                ("notification_type", "toast"),
            ]),
            row(&[
                ("id", "2"),
                ("first_seen_utc", "2026-09-29T11:00:00.000000Z"),
                ("app_name", "Chat"),
                ("title", "<b>жирный</b>"),
                ("body", "строка"),
                ("seen_count", "1"),
                ("revision", "1"),
            ]),
        ]
    }

    #[test]
    fn formats_are_recognised_and_named() {
        assert_eq!(Format::from_name("json"), Format::Json);
        assert_eq!(Format::from_name("HTML"), Format::Html);
        assert_eq!(Format::from_name("unknown"), Format::Csv);
        assert_eq!(
            Format::from_path(Path::new(r"D:\a\b.HTML")),
            Some(Format::Html)
        );
        assert_eq!(Format::from_path(Path::new("b.htm")), Some(Format::Html));
        assert_eq!(Format::from_path(Path::new("b.txt")), Some(Format::Txt));
        assert_eq!(Format::from_path(Path::new("b")), None);

        // The name offered in the save dialog follows the chosen format ...
        assert_eq!(
            suggested_name(Format::Csv, "20260929-214735"),
            "notifications-20260929-214735.csv"
        );
        assert_eq!(suggested_name(Format::Json, "x"), "notifications-x.json");
        // ... and the file dialog offers every format plus "all files".
        assert!(filters()
            .iter()
            .any(|(caption, _)| caption.contains("JSON")));
        assert_eq!(
            filters().last().map(|(_, pattern)| pattern.as_str()),
            Some("*.*")
        );
    }

    #[test]
    fn csv_quotes_only_what_needs_quoting() {
        assert_eq!(csv_field("просто"), "просто");
        assert_eq!(csv_field("a;b"), "\"a;b\"");
        assert_eq!(csv_field("он сказал \"да\""), "\"он сказал \"\"да\"\"\"");
        assert_eq!(csv_field("первая\nвторая"), "\"первая\nвторая\"");
    }

    #[test]
    fn csv_starts_with_a_byte_order_mark_and_one_line_per_record() {
        let text = render(&sample(), Format::Csv, "стоп");
        assert!(text.starts_with('\u{feff}'));
        let lines: Vec<&str> = text.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines.len(), 3, "one header plus two records");
        assert!(lines[0].starts_with("id;utc;local;app"));
        assert!(lines[1].contains("Почта"));
        assert!(lines[1].contains("\"Привет; \"\"кавычки\"\"\""));
    }

    #[test]
    fn json_is_valid_and_counts_the_records() {
        let text = render(&sample(), Format::Json, "стоп");
        let json: Json = serde_json::from_str(&text).expect("valid json");
        assert_eq!(json["count"], json!(2));
        assert_eq!(json["notifications"][0]["app"], json!("Почта"));
        assert_eq!(json["notifications"][1]["seen_count"], json!(1));
        assert!(json["exported_local"].is_string());
    }

    #[test]
    fn text_export_is_readable() {
        let text = render(&sample(), Format::Txt, "29.09.2026 21:00:00");
        assert!(text.contains("Записей: 2"));
        assert!(text.contains("Почта"));
        assert!(text.contains("Заголовок: Новое письмо"));
        assert!(
            !text.contains("Время: "),
            "the time is already in the heading"
        );
    }

    #[test]
    fn html_escapes_markup_and_keeps_structure() {
        let text = render(&sample(), Format::Html, "стоп");
        assert!(text.starts_with("<!DOCTYPE html>"));
        assert!(text.contains("&lt;b&gt;жирный&lt;/b&gt;"));
        assert!(!text.contains("<b>жирный</b>"));
        assert_eq!(text.matches("class=\"card\"").count(), 2);
        assert_eq!(
            escape_html("a & b < c > d \"e\""),
            "a &amp; b &lt; c &gt; d &quot;e&quot;"
        );
    }
}
