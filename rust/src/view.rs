//! Presentation helpers shared by the list, the details window and the export.
//!
//! Everything here is a pure function of a journal row, which keeps the visual
//! rules in one place and testable without a window: the columns of the list, the
//! readable form of a notification, the technical fields that stay hidden by
//! default and the text of the clipboard or the exported file.

use crate::sqlite::Row;
use crate::time;

/// One column of the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    /// Stable key, also used to pick the sort order.
    pub key: &'static str,
    pub title: &'static str,
    /// Default width in logical pixels (96 DPI).
    pub width: i32,
    /// Right aligned (numbers) instead of left aligned.
    pub right: bool,
    /// Shown only when the user asked for technical columns.
    pub technical: bool,
}

/// The friendly columns: what a normal user wants to see.
pub const BASE_COLUMNS: [Column; 4] = [
    Column {
        key: "time",
        title: "Время",
        width: 150,
        right: false,
        technical: false,
    },
    Column {
        key: "app",
        title: "Приложение",
        width: 210,
        right: false,
        technical: false,
    },
    Column {
        key: "title",
        title: "Заголовок",
        width: 260,
        right: false,
        technical: false,
    },
    Column {
        key: "text",
        title: "Текст",
        width: 460,
        right: false,
        technical: false,
    },
];

/// Everything the collectors can tell us, for people who want the details.
pub const TECHNICAL_COLUMNS: [Column; 5] = [
    Column {
        key: "id",
        title: "№",
        width: 64,
        right: true,
        technical: true,
    },
    Column {
        key: "kind",
        title: "Тип",
        width: 90,
        right: false,
        technical: true,
    },
    Column {
        key: "sources",
        title: "Источник",
        width: 90,
        right: false,
        technical: true,
    },
    Column {
        key: "seen",
        title: "Повторов",
        width: 84,
        right: true,
        technical: true,
    },
    Column {
        key: "aumid",
        title: "Идентификатор",
        width: 240,
        right: false,
        technical: true,
    },
];

/// Every column the feed can show, in the order the menu offers them.
///
/// The feed is meant to be read like a chat window, so it keeps the smallest set of
/// columns that still says who sent what: the source and the content.  The other
/// two are there for a reader who wants them (`Settings::feed_columns`) - a time in
/// front of every line, or the title apart from the message.
///
/// The icon or picture of a row sits in front of the *first* column - that is where
/// a report view draws its image - which is why the source comes before the content
/// in the default choice.
pub const FEED_COLUMNS: [Column; 4] = [
    Column {
        key: "time",
        title: "Время",
        width: 130,
        right: false,
        technical: false,
    },
    Column {
        key: "app",
        // The window narrows or widens this column to its own client area (see
        // `ui::App::apply_column_widths`); the value here is what a feed shows
        // before it has been laid out once.
        title: "Источник",
        width: 200,
        right: false,
        technical: false,
    },
    Column {
        key: "title",
        title: "Заголовок",
        width: 240,
        right: false,
        technical: false,
    },
    Column {
        key: "content",
        title: "Содержание",
        width: 460,
        right: false,
        technical: false,
    },
];

/// The keys of [`FEED_COLUMNS`], in the order a reader sees them.
pub const FEED_COLUMN_KEYS: [&str; 4] = ["time", "app", "title", "content"];

/// The columns a feed shows until the reader picks others: source, then content.
pub fn default_feed_column_keys() -> Vec<String> {
    vec!["app".to_string(), "content".to_string()]
}

/// The columns of the feed: the ones the reader picked, in the order of
/// [`FEED_COLUMNS`].
///
/// An empty or unknown choice falls back on [`default_feed_column_keys`] - a list
/// without columns would show nothing at all.
pub fn feed_columns(keys: &[String]) -> Vec<Column> {
    let picked: Vec<Column> = FEED_COLUMNS
        .iter()
        .filter(|column| keys.iter().any(|key| key == column.key))
        .copied()
        .collect();
    if picked.is_empty() {
        return feed_columns(&default_feed_column_keys());
    }
    picked
}

/// Columns for the requested view: friendly, or friendly plus technical.
pub fn columns(technical: bool) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::with_capacity(BASE_COLUMNS.len() + TECHNICAL_COLUMNS.len());
    columns.push(BASE_COLUMNS[0]);
    if technical {
        columns.push(TECHNICAL_COLUMNS[0]); // the number belongs in front
    }
    columns.extend_from_slice(&BASE_COLUMNS[1..]);
    if technical {
        columns.extend_from_slice(&TECHNICAL_COLUMNS[1..]);
    }
    columns
}

/// Sort key of a column (`SearchQuery::sort`).
pub fn sort_key(column: &Column) -> &'static str {
    match column.key {
        "id" => "id",
        "app" => "app",
        "aumid" => "aumid",
        "title" => "title",
        "kind" => "kind",
        "sources" => "sources",
        "seen" => "seen",
        // The feed shows the content of a notification; it sorts like the title.
        "content" => "title",
        _ => "time",
    }
}

/// First address a notification carries, if any.
///
/// Notification payloads put links in the `launch` attribute or into an action
/// argument (`arguments="https://…"`), and both end up readable in the row; the
/// context menu offers "open this link", so the extraction lives here next to
/// the other presentation rules.
pub fn first_link(row: &Row) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(launch) = row.optional_text("launch") {
        candidates.push(launch);
    }
    for action in actions(row) {
        candidates.push(action);
    }
    for candidate in candidates {
        if let Some(link) = find_address(&candidate) {
            return Some(link);
        }
    }
    None
}

/// Pull the first `http(s)://…` or `file://…` address out of a text.
pub fn find_address(text: &str) -> Option<String> {
    let start = ["https://", "http://", "file://"]
        .iter()
        .filter_map(|prefix| text.find(prefix))
        .min()?;
    let rest = &text[start..];
    let end = rest
        .find(|character: char| character.is_whitespace() || matches!(character, '»' | '"' | '\''))
        .unwrap_or(rest.len());
    let address = rest[..end].trim_end_matches(['.', ',', ';']).to_string();
    (address.len() > 8).then_some(address)
}

/// Time range offered by the *Период* filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Everything in the journal.
    All,
    /// From local midnight today.
    Today,
    /// The last seven days.
    Week,
    /// The last thirty days.
    Month,
    /// The two date fields under the list.
    Custom,
}

/// Entries of the period list, in the order they are shown.
pub const PERIODS: [(Period, &str); 5] = [
    (Period::All, "Всё время"),
    (Period::Today, "Сегодня"),
    (Period::Week, "Последние 7 дней"),
    (Period::Month, "Последние 30 дней"),
    (Period::Custom, "Свой диапазон"),
];

/// Position of a period inside `PERIODS` (for the combo box).
pub fn period_index(period: Period) -> usize {
    PERIODS
        .iter()
        .position(|(value, _)| *value == period)
        .unwrap_or(0)
}

/// Period at a position of the combo box.
pub fn period_at(index: usize) -> Period {
    PERIODS
        .get(index)
        .map(|(period, _)| *period)
        .unwrap_or(Period::All)
}

/// Lower bound a preset period implies (`None` when everything is included).
///
/// Only the lower bound is needed: a preset always ends "now", and a
/// notification cannot be recorded in the future.
pub fn period_since(period: Period, now_micros: i64) -> Option<String> {
    const DAY_MICROS: i64 = 86_400_000_000;
    match period {
        Period::All | Period::Custom => None,
        Period::Today => Some(time::local_day_start_iso(now_micros)),
        Period::Week => Some(time::micros_to_iso(now_micros - 7 * DAY_MICROS)),
        Period::Month => Some(time::micros_to_iso(now_micros - 30 * DAY_MICROS)),
    }
}

/// Bounds of the custom range typed into the two date fields.
///
/// An unreadable field is ignored instead of rejecting the whole filter, so a
/// half-filled range still narrows the list in one direction.
pub fn custom_range(since: &str, until: &str) -> (Option<String>, Option<String>) {
    let from = if since.trim().is_empty() {
        None
    } else {
        time::local_date_to_iso(since, false)
    };
    let to = if until.trim().is_empty() {
        None
    } else {
        time::local_date_to_iso(until, true)
    };
    (from, to)
}

/// Application name as shown to the user.
///
/// The name a collector or the registry reported comes first, and only when there
/// is none the identifier itself is shown - in its readable form, so that a
/// Chromium browser does not appear as `Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456` in one
/// row and as `Thorium` in the next one.
pub fn display_app(row: &Row) -> String {
    row.optional_text("app_name")
        .or_else(|| {
            row.optional_text("aumid")
                .map(|aumid| crate::appname::readable_identifier(&aumid).to_string())
        })
        .unwrap_or_else(|| "(без имени)".to_string())
}

/// Local time of a journal timestamp as `DD.MM.YYYY HH:MM:SS`.
pub fn local_time(utc: Option<&str>) -> String {
    time::format_local_iso(utc)
}

/// Collapse whitespace so a value fits on a single listing line.
pub fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The content of a notification in one line: what the text columns of the list
/// show, what the text export writes under the heading and what the feed file
/// holds after the time and the source.
///
/// The title comes first, then the body; when only one of them is there it
/// stands alone, and a notification that carries no text at all falls back on
/// the extra lines the collectors picked up.
pub fn row_text(row: &Row) -> String {
    let title = row.optional_text("title").unwrap_or_default();
    let body = row.optional_text("body").unwrap_or_default();
    let extra = row.optional_text("extra_text").unwrap_or_default();
    if !title.trim().is_empty() && !body.trim().is_empty() {
        format!("{title} — {body}")
    } else if !title.trim().is_empty() {
        title
    } else if !body.trim().is_empty() {
        body
    } else {
        extra
    }
}

/// Shorten a single line value to `limit` characters.
pub fn flatten(text: &str, limit: usize) -> String {
    let line = single_line(text);
    if line.chars().count() <= limit {
        return line;
    }
    let mut shortened: String = line.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// Decode a JSON array of strings; objects (notification actions) are rendered
/// as `"подпись" [тип] аргумент`.
pub fn json_list(value: Option<String>) -> Vec<String> {
    value
        .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
        .map(|items| {
            items
                .iter()
                .map(|item| match item.as_str() {
                    Some(text) => text.to_string(),
                    None => action_line(item),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Human readable form of one notification action.
pub fn action_line(item: &serde_json::Value) -> String {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let content = {
        let content = text("content");
        if content.is_empty() {
            text("label")
        } else {
            content
        }
    };
    let mut parts: Vec<String> = Vec::new();
    if !content.is_empty() {
        parts.push(format!("\u{201c}{content}\u{201d}"));
    }
    let kind = text("type");
    if !kind.is_empty() {
        parts.push(format!("[{kind}]"));
    }
    let arguments = text("arguments");
    if !arguments.is_empty() {
        parts.push(arguments);
    }
    if parts.is_empty() {
        item.to_string()
    } else {
        parts.join(" ")
    }
}

/// Pictures a notification carried (usually `ms-appx://` or file references).
///
/// Empty references are dropped: a payload with `<image src=""/>` is not a
/// picture, and a list of them would only produce empty lines.
pub fn images(row: &Row) -> Vec<String> {
    json_list(row.optional_text("images"))
        .into_iter()
        .filter(|reference| !reference.trim().is_empty())
        .collect()
}

/// Actions a notification offered.
pub fn actions(row: &Row) -> Vec<String> {
    json_list(row.optional_text("actions"))
}

/// What the list shows: one cell per column.
pub fn row_cells(row: &Row, columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .map(|column| match column.key {
            "id" => row.int("id").unwrap_or_default().to_string(),
            "time" => local_time(row.text("first_seen_utc").as_deref()),
            "app" => display_app(row),
            "title" => flatten(row.optional_text("title").unwrap_or_default().as_str(), 200),
            "text" => flatten(row.optional_text("body").unwrap_or_default().as_str(), 400),
            // The feed usually has no separate title column, so its text column
            // holds what the notification actually said.  When the title *is* shown
            // in a column of its own, this one carries the rest of the message
            // instead of repeating the first words of it.
            "content" => {
                let body_only = columns.iter().any(|other| other.key == "title");
                let text = if body_only {
                    row.optional_text("body").unwrap_or_default()
                } else {
                    row_text(row)
                };
                flatten(&text, 600)
            }
            "kind" => row.text("notification_type").unwrap_or_default(),
            "sources" => row.text("sources").unwrap_or_default(),
            "seen" => row.int("seen_count").unwrap_or_default().to_string(),
            "aumid" => row.optional_text("aumid").unwrap_or_default(),
            _ => String::new(),
        })
        .collect()
}

/// Fields a reader cares about: label and value.
pub fn readable_lines(row: &Row) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.trim().is_empty() {
            lines.push((label.to_string(), value));
        }
    };
    push(tr!("Время"), local_time(row.text("first_seen_utc").as_deref()));
    push(tr!("Приложение"), display_app(row));
    push(tr!("Заголовок"), row.optional_text("title").unwrap_or_default());
    push(tr!("Текст"), row.optional_text("body").unwrap_or_default());
    let extra: Vec<String> = json_list(row.optional_text("extra_text"));
    if !extra.is_empty() {
        push(tr!("Дополнительно"), extra.join("\n"));
    }
    let actions = actions(row);
    if !actions.is_empty() {
        push(tr!("Действия"), actions.join("\n"));
    }
    if let Some(launch) = row.optional_text("launch") {
        push(tr!("Ссылка"), launch);
    }
    if let Some(last) = row.optional_text("last_seen_utc") {
        if row.text("first_seen_utc").as_deref() != Some(last.as_str()) {
            push(tr!("Последний раз"), local_time(Some(&last)));
        }
    }
    lines
}

/// Fields only an administrator or a developer wants: hidden behind a switch.
pub fn technical_lines(row: &Row) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.trim().is_empty() {
            lines.push((label.to_string(), value));
        }
    };
    push(tr!("№"), row.int("id").unwrap_or_default().to_string());
    push(
        tr!("Тип уведомления"),
        row.text("notification_type").unwrap_or_default(),
    );
    push(
        tr!("Идентификатор приложения"),
        row.optional_text("aumid").unwrap_or_default(),
    );
    push(tr!("Источники"), row.text("sources").unwrap_or_default());
    push(
        tr!("Повторов"),
        tf!(
            "{} (ревизия {})",
            "{} (revision {})",
            row.int("seen_count").unwrap_or_default(),
            row.int("revision").unwrap_or_default()
        ),
    );
    push(tr!("Тег"), row.optional_text("tag").unwrap_or_default());
    push(tr!("Группа"), row.optional_text("grp").unwrap_or_default());
    push(
        tr!("Сценарий"),
        row.optional_text("scenario").unwrap_or_default(),
    );
    push(
        tr!("Тип активации"),
        row.optional_text("activation_type").unwrap_or_default(),
    );
    push(
        tr!("Обработчик"),
        row.optional_text("handler_type").unwrap_or_default(),
    );
    push(tr!("Ключ"), row.text("dedup_key").unwrap_or_default());
    push(
        tr!("Хеш содержимого"),
        row.text("content_hash").unwrap_or_default(),
    );
    push(
        "Payload",
        row.optional_text("payload_xml").unwrap_or_default(),
    );
    lines
}

/// One picture reference of a notification (empty when there is none).
pub fn first_image(row: &Row) -> Option<String> {
    images(row).into_iter().next()
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

    #[test]
    fn columns_come_first_and_cells_follow_them() {
        let base = columns(false);
        assert_eq!(base.len(), BASE_COLUMNS.len());
        assert!(base.iter().all(|column| !column.technical));
        assert_eq!(base[0].title, "Время");
        assert_eq!(base[1].title, "Приложение");

        let all = columns(true);
        assert_eq!(all.len(), BASE_COLUMNS.len() + TECHNICAL_COLUMNS.len());
        assert_eq!(all[0].key, "time");
        assert_eq!(all[1].key, "id", "the number sits before the time");
        assert!(all.iter().any(|column| column.key == "aumid"));

        // The same set of columns decides what one row shows.
        let row = row(&[
            ("id", "7"),
            ("first_seen_utc", "2026-09-29T10:11:12.000000Z"),
            ("app_name", "Почта"),
            ("title", "Новое письмо"),
            ("body", "Привет!\n\nКак дела?"),
            ("seen_count", "2"),
            ("notification_type", "toast"),
            ("sources", "wpndb"),
            ("aumid", "Vendor.Mail_1!App"),
        ]);
        let base = row_cells(&row, &columns(false));
        assert_eq!(base.len(), 4);
        assert_eq!(base[1], "Почта");
        assert_eq!(base[2], "Новое письмо");
        assert_eq!(
            base[3], "Привет! Как дела?",
            "newlines collapse into spaces"
        );

        let all = row_cells(&row, &columns(true));
        assert_eq!(all.len(), 9);
        // Order: время, №, приложение, заголовок, текст, тип, источник, повторов, id.
        assert_eq!(all[0], base[0], "the time column stays first");
        assert_eq!(all[1], "7");
        assert_eq!(all[2], "Почта");
        assert_eq!(all[5], "toast");
        assert_eq!(all[6], "wpndb");
        assert_eq!(all[7], "2");
        assert_eq!(all[8], "Vendor.Mail_1!App");
    }

    #[test]
    fn missing_values_do_not_produce_empty_labels() {
        let row = row(&[("id", "1"), ("first_seen_utc", "")]);
        let readable = readable_lines(&row);
        assert!(readable.iter().all(|(_, value)| !value.trim().is_empty()));
    }

    #[test]
    fn technical_lines_are_separate_from_the_readable_ones() {
        let row = row(&[
            ("id", "3"),
            ("title", "Заголовок"),
            ("dedup_key", "key-1"),
            ("seen_count", "5"),
            ("revision", "2"),
        ]);
        let readable = readable_lines(&row);
        assert!(readable.iter().any(|(label, _)| label == "Заголовок"));
        assert!(!readable.iter().any(|(label, _)| label == "Ключ"));

        let technical = technical_lines(&row);
        assert!(technical.iter().any(|(label, _)| label == "Ключ"));
        assert!(technical
            .iter()
            .any(|(label, value)| label == "Повторов" && value.contains('5')));
    }

    #[test]
    fn actions_and_images_are_readable() {
        let row = row(&[
            (
                "actions",
                r#"[{"content":"Открыть","type":"foreground","arguments":"app://open"}]"#,
            ),
            ("images", r#"["", "ms-appx:///Assets/logo.png", "  ", "https://a/b.png"]"#),
        ]);
        assert_eq!(
            actions(&row),
            vec!["“Открыть” [foreground] app://open".to_string()]
        );
        assert_eq!(
            images(&row),
            vec![
                "ms-appx:///Assets/logo.png".to_string(),
                "https://a/b.png".to_string()
            ],
            "empty references are not pictures"
        );
        assert_eq!(
            first_image(&row).as_deref(),
            Some("ms-appx:///Assets/logo.png")
        );
    }

    #[test]
    fn sorting_follows_the_column() {
        let all = columns(true);
        assert_eq!(sort_key(&all[0]), "time");
        assert_eq!(sort_key(&all[1]), "id");
        let app = all
            .iter()
            .find(|column| column.key == "app")
            .expect("app column");
        assert_eq!(sort_key(app), "app");
        // Every column has to sort by something meaningful, never silently by time.
        for column in &all {
            assert!(
                !sort_key(column).is_empty(),
                "column {} has no sort key",
                column.key
            );
        }
    }

    #[test]
    fn links_are_taken_from_launch_and_actions() {
        let from_launch = row(&[("launch", "https://example.org/mail")]);
        assert_eq!(
            first_link(&from_launch).as_deref(),
            Some("https://example.org/mail")
        );

        let from_action = row(&[(
            "actions",
            r#"[{"content":"Открыть сайт","arguments":"https://example.org/x?y=1&z=2"}]"#,
        )]);
        assert_eq!(
            first_link(&from_action).as_deref(),
            Some("https://example.org/x?y=1&z=2"),
            "the address stops before the closing quote"
        );

        let nothing = row(&[("body", "просто текст без ссылок")]);
        assert_eq!(first_link(&nothing), None);

        assert_eq!(
            find_address("смотри https://a.bc/def.»"),
            Some("https://a.bc/def".to_string()),
            "the closing bracket and the full stop are not part of the address"
        );
        assert_eq!(find_address("нет ссылки"), None);
        assert_eq!(find_address("http://a"), None, "too short to be a link");
    }

    #[test]
    fn periods_pick_the_expected_bounds() {
        // 2026-09-29 12:00:00 UTC as the reference moment.
        let now = 1_790_798_400_000_000;
        assert_eq!(period_since(Period::All, now), None);
        assert_eq!(period_since(Period::Custom, now), None);

        let week = period_since(Period::Week, now).expect("week");
        assert!(week < time::micros_to_iso(now));
        let today = period_since(Period::Today, now).expect("today");
        assert!(today > week, "midnight today is later than a week ago");

        assert_eq!(period_index(Period::All), 0);
        assert_eq!(period_at(0), Period::All);
        assert_eq!(period_at(period_index(Period::Month)), Period::Month);
        assert_eq!(period_at(99), Period::All, "an unknown index falls back");
        assert_eq!(PERIODS.len(), 5);
    }

    #[test]
    fn a_custom_range_survives_half_filled_fields() {
        let (from, to) = custom_range("29.09.2026", "");
        assert!(from.is_some());
        assert_eq!(to, None);

        let (from, to) = custom_range("", "29.09.2026");
        assert_eq!(from, None);
        assert!(to.is_some());

        let (from, to) = custom_range("вчера", "завтра");
        assert_eq!(
            (from, to),
            (None, None),
            "both fields ignored when unreadable"
        );
    }

    /// A browser that registered no name shows up as `Thorium` instead of
    /// `Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456` - and the identifier keeps every letter
    /// of its own in the technical column.
    #[test]
    fn an_unnamed_application_shows_a_readable_identifier() {
        let hashed = row(&[("aumid", "Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456")]);
        assert_eq!(display_app(&hashed), "Thorium");
        let columns = columns(true);
        let position = columns
            .iter()
            .position(|column| column.key == "aumid")
            .expect("technical column");
        assert_eq!(
            row_cells(&hashed, &columns)[position],
            "Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456",
            "the identifier itself is never rewritten"
        );

        // A resolved name still wins over the identifier.
        let named = row(&[
            ("aumid", "Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"),
            ("app_name", "Thorium"),
        ]);
        assert_eq!(display_app(&named), "Thorium");

        // A row without both keeps its place in the list.
        assert_eq!(display_app(&row(&[])), "(без имени)");
    }

    /// The feed is the smallest form of the list: who sent it, what it said, and
    /// nothing beside that.  A column that sneaks in shows up here first.
    #[test]
    fn the_feed_keeps_the_source_and_the_content_only() {
        let feed = feed_columns(&default_feed_column_keys());
        assert_eq!(feed.len(), 2, "the feed starts minimal");
        assert_eq!(feed[0].key, "app");
        assert_eq!(feed[1].key, "content");
        assert!(
            feed.iter().all(|column| !column.technical),
            "the feed shows no technical columns"
        );
        for column in &feed {
            assert!(
                !sort_key(column).is_empty(),
                "column {} has no sort key",
                column.key
            );
        }

        let row = row(&[
            ("app_name", "Почта"),
            ("title", "Новое письмо"),
            ("body", "Привет!\n\nКак дела?"),
        ]);
        let cells = row_cells(&row, &feed);
        assert_eq!(cells[0], "Почта");
        assert_eq!(
            cells[1], "Новое письмо — Привет! Как дела?",
            "the single text column carries title and body"
        );
    }

    /// The reader decides which columns the feed shows; the time and the title are
    /// there to be added, and the choice never produces an empty list.
    #[test]
    fn the_feed_columns_follow_the_readers_choice() {
        let keys = |list: &[&str]| -> Vec<String> {
            list.iter().map(|key| (*key).to_string()).collect()
        };

        let chosen = feed_columns(&keys(&["time", "app", "content"]));
        assert_eq!(
            chosen.iter().map(|column| column.key).collect::<Vec<_>>(),
            vec!["time", "app", "content"],
            "the order is the one of the menu, not the one of the file"
        );

        // An unknown key is ignored, so a hand edited file stays harmless.
        let unknown = feed_columns(&keys(&["app", "nonsense"]));
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].key, "app");

        // Nothing chosen at all: the default pair, never an empty table.
        let fallback = feed_columns(&[]);
        assert_eq!(fallback.len(), 2);
        assert_eq!(fallback[0].key, "app");
        assert_eq!(fallback[1].key, "content");

        // Every offered key really has a column.
        for key in FEED_COLUMN_KEYS {
            let columns = feed_columns(&keys(&[key]));
            assert_eq!(columns.len(), 1, "no column for {key}");
            assert_eq!(columns[0].key, key);
            assert!(!columns[0].title.is_empty(), "{key} has no caption");
        }

        // The menu offers `FEED_COLUMNS` in their own order and takes the key of a
        // column from `FEED_COLUMN_KEYS` by position, so the two have to agree.
        assert_eq!(
            FEED_COLUMN_KEYS.len(),
            FEED_COLUMNS.len(),
            "every column needs a key and every key a column"
        );
        for (key, column) in FEED_COLUMN_KEYS.iter().zip(FEED_COLUMNS.iter()) {
            assert_eq!(*key, column.key, "the keys are in the order of the columns");
        }

        // With the title in a column of its own the content column does not repeat
        // it: the reader sees every word once.
        let row = row(&[
            ("app_name", "Почта"),
            ("title", "Новое письмо"),
            ("body", "Привет!\n\nКак дела?"),
        ]);
        let both = feed_columns(&keys(&["title", "content"]));
        let cells = row_cells(&row, &both);
        assert_eq!(cells[0], "Новое письмо");
        assert_eq!(cells[1], "Привет! Как дела?");
    }
}
