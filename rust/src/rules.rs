//! Rules for incoming notifications: what is kept, what is marked, what is thrown
//! away and what is worth a pop-up.
//!
//! These rules are the extension point the specification asks for, and this module
//! is the same engine as `winnotifylog/filters.py` in the Python implementation:
//! the field names, the actions and the matching are identical, so both programs
//! read the same rule list out of their settings file.
//!
//! A rule selects notifications by application, by kind, by collector and by text,
//! and decides what happens with them:
//!
//! * `store`  - keep the notification in the journal (the default);
//! * `flag`   - keep it and mark it with a label (the label lands in the extra
//!              data of the record, where the details window shows it);
//! * `skip`   - do not store it at all (for example an application that notifies
//!              once a second about nothing);
//! * `alert`  - keep it and show a pop-up (the action the desktop version adds: a
//!              program that collects silently in the background is easy to
//!              forget, and some notifications are worth seeing at once).
//!
//! Rules are evaluated in order and the first match wins.  Comparison ignores
//! case, and a pattern matches when it is a wildcard pattern (`*`, `?`) or when it
//! is simply contained in the value - "Steam" finds both `Steam.Desktop` and
//! `Магазин Steam`.

use serde_json::Value as Json;

use crate::model::NotificationRecord;

/// Keep the notification in the journal.
pub const ACTION_STORE: &str = "store";
/// Keep the notification and mark it with a label.
pub const ACTION_FLAG: &str = "flag";
/// Do not store the notification.
pub const ACTION_SKIP: &str = "skip";
/// Keep the notification and show a pop-up.
pub const ACTION_ALERT: &str = "alert";

/// Every action a rule may carry.
pub const ACTIONS: [&str; 4] = [ACTION_STORE, ACTION_FLAG, ACTION_SKIP, ACTION_ALERT];

/// One rule of notification processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// Name of the rule; the log and the label fall back to it.
    pub name: String,
    pub enabled: bool,
    /// Application user model ids to match; empty means every application.
    pub apps: Vec<String>,
    /// Patterns matched against the title, the body and the extra lines.
    pub text: Vec<String>,
    /// Notification kinds: `toast`, `tile`, `badge`.
    pub types: Vec<String>,
    /// Collectors: `wpndb`, `uns`.
    pub sources: Vec<String>,
    /// One of [`ACTIONS`].
    pub action: String,
    /// Label written into the record when the action is `flag` or `alert`.
    pub label: Option<String>,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            name: "rule".to_string(),
            enabled: true,
            apps: Vec::new(),
            text: Vec::new(),
            types: Vec::new(),
            sources: Vec::new(),
            action: ACTION_STORE.to_string(),
            label: None,
        }
    }
}
impl Rule {
    /// Does this rule apply to the record?
    pub fn matches(&self, record: &NotificationRecord) -> bool {
        if !self.apps.is_empty() {
            let value = record
                .aumid
                .as_deref()
                .or(record.app_name.as_deref())
                .unwrap_or_default();
            if !matches_any(value, &self.apps) {
                return false;
            }
        }
        if !self.types.is_empty() {
            let kind = record.notification_type.trim().to_lowercase();
            if !self
                .types
                .iter()
                .any(|item| item.trim().to_lowercase() == kind)
            {
                return false;
            }
        }
        if !self.sources.is_empty() {
            let source = record.source.to_lowercase();
            if !self.sources.iter().any(|item| {
                !item.trim().is_empty() && source.contains(&item.trim().to_lowercase())
            }) {
                return false;
            }
        }
        if !self.text.is_empty() && !matches_any(&searchable_text(record), &self.text) {
            return false;
        }
        true
    }
}

/// What the collector should do with a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// `false` means the notification is not written to the journal at all.
    pub store: bool,
    /// Label of the matching rule.
    pub label: Option<String>,
    /// Name of the rule that decided; `None` when no rule matched.
    pub rule: Option<String>,
    /// Show a pop-up for this notification.
    pub alert: bool,
}

impl Default for Decision {
    fn default() -> Self {
        Decision::keep()
    }
}

impl Decision {
    /// The default decision: keep everything, say nothing.
    pub fn keep() -> Decision {
        Decision {
            store: true,
            label: None,
            rule: None,
            alert: false,
        }
    }

    /// Did a rule throw this notification away?
    pub fn is_skipped(&self) -> bool {
        !self.store
    }
}

/// The ordered set of rules out of the settings file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet {
    rules: Vec<Rule>,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> RuleSet {
        RuleSet { rules }
    }

    /// Read the rules out of the `rules` array of the settings file.
    ///
    /// A rule that cannot be understood is noted and dropped rather than breaking
    /// the list: a typo in one rule must not stop the collector, which is what a
    /// program running in the background has to guarantee.
    pub fn from_json(value: &Json) -> RuleSet {
        let mut rules = Vec::new();
        let Some(items) = value.as_array() else {
            return RuleSet::default();
        };
        for (index, item) in items.iter().enumerate() {
            let Some(object) = item.as_object() else {
                eprintln!("[rules] правило #{} не объект и пропущено", index + 1);
                continue;
            };
            let name = object
                .get("name")
                .and_then(Json::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("rule-{}", index + 1));
            let action = object
                .get("action")
                .and_then(Json::as_str)
                .map(|value| value.trim().to_lowercase())
                .unwrap_or_else(|| ACTION_STORE.to_string());
            let action = if ACTIONS.contains(&action.as_str()) {
                action
            } else {
                eprintln!("[rules] правило {name}: неизвестное действие «{action}», принято store");
                ACTION_STORE.to_string()
            };
            rules.push(Rule {
                name,
                enabled: object.get("enabled").and_then(Json::as_bool).unwrap_or(true),
                apps: strings(object.get("apps")),
                text: strings(object.get("text")),
                types: strings(object.get("types")),
                sources: strings(object.get("sources")),
                action,
                label: object
                    .get("label")
                    .and_then(Json::as_str)
                    .map(str::to_string)
                    .filter(|value| !value.trim().is_empty()),
            });
        }
        RuleSet { rules }
    }

    /// The JSON form written back to the settings file.
    pub fn to_json(&self) -> Json {
        Json::Array(
            self.rules
                .iter()
                .map(|rule| {
                    let mut object = serde_json::Map::new();
                    object.insert("name".into(), Json::String(rule.name.clone()));
                    object.insert("enabled".into(), Json::Bool(rule.enabled));
                    object.insert("action".into(), Json::String(rule.action.clone()));
                    for (key, values) in [
                        ("apps", &rule.apps),
                        ("text", &rule.text),
                        ("types", &rule.types),
                        ("sources", &rule.sources),
                    ] {
                        if !values.is_empty() {
                            object.insert(key.into(), Json::from(values.clone()));
                        }
                    }
                    if let Some(label) = &rule.label {
                        object.insert("label".into(), Json::String(label.clone()));
                    }
                    Json::Object(object)
                })
                .collect(),
        )
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// How many rules are switched on.
    pub fn enabled_count(&self) -> usize {
        self.rules.iter().filter(|rule| rule.enabled).count()
    }

    /// One line for the log: which rules are in force.
    pub fn describe(&self) -> String {
        if self.rules.is_empty() {
            return tr!("правил нет").to_string();
        }
        let active: Vec<String> = self
            .rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| format!("{}:{}", rule.name, rule.action))
            .collect();
        if active.is_empty() {
            tr!("все правила выключены").to_string()
        } else {
            active.join(", ")
        }
    }

    /// The decision for one record: the first matching rule wins.
    pub fn evaluate(&self, record: &NotificationRecord) -> Decision {
        for rule in &self.rules {
            if !rule.enabled || !rule.matches(record) {
                continue;
            }
            // The label falls back to the name of the rule, so a rule that marks
            // notifications can be recognised even without an invented label.
            let label = rule
                .label
                .clone()
                .or_else(|| (!rule.name.trim().is_empty()).then(|| rule.name.clone()));
            return match rule.action.as_str() {
                ACTION_SKIP => Decision {
                    store: false,
                    label: rule.label.clone(),
                    rule: Some(rule.name.clone()),
                    alert: false,
                },
                ACTION_ALERT => Decision {
                    store: true,
                    label,
                    rule: Some(rule.name.clone()),
                    alert: true,
                },
                _ => Decision {
                    store: true,
                    label,
                    rule: Some(rule.name.clone()),
                    alert: false,
                },
            };
        }
        Decision::keep()
    }
}

/// A pop-up a rule asked for.
///
/// It travels through `status.json` from the collector thread to the window: the
/// background thread has no user interface, and the balloon belongs to the tray
/// icon the window owns.  `seq` is what tells the window which of them are new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub seq: u64,
    pub application: String,
    pub title: String,
    pub body: String,
    pub label: Option<String>,
}

impl Alert {
    pub fn to_json(&self) -> Json {
        serde_json::json!({
            "seq": self.seq,
            "application": self.application,
            "title": self.title,
            "body": self.body,
            "label": self.label,
        })
    }

    /// Tolerant reading of one alert; an alert without a number is not usable,
    /// because the window could not tell whether it is new.
    pub fn from_json(json: &Json) -> Option<Alert> {
        let seq = json.get("seq").and_then(Json::as_u64)?;
        let text = |key: &str| {
            json.get(key)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Some(Alert {
            seq,
            application: text("application"),
            title: text("title"),
            body: text("body"),
            label: json
                .get("label")
                .and_then(Json::as_str)
                .map(str::to_string)
                .filter(|value| !value.is_empty()),
        })
    }

    /// One line for the balloon body.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.application.trim().is_empty() {
            parts.push(self.application.trim().to_string());
        }
        if !self.body.trim().is_empty() {
            parts.push(self.body.trim().to_string());
        }
        let text = parts.join(" · ");
        if text.is_empty() {
            "без текста".to_string()
        } else if text.chars().count() > 200 {
            format!("{}…", text.chars().take(199).collect::<String>())
        } else {
            text
        }
    }

    /// Heading of the balloon: the title, or the application when there is none.
    pub fn heading(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.chars().take(63).collect();
        }
        let application = self.application.trim();
        if application.is_empty() {
            "Уведомление".to_string()
        } else {
            application.chars().take(63).collect()
        }
    }
}

/// Text a rule can look at: everything a person reads in the list.
fn searchable_text(record: &NotificationRecord) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for value in [&record.title, &record.body] {
        if let Some(text) = value {
            parts.push(text);
        }
    }
    for text in &record.extra_text {
        parts.push(text);
    }
    for value in [&record.app_name, &record.aumid] {
        if let Some(text) = value {
            parts.push(text);
        }
    }
    parts.join(" ")
}

/// Does any pattern match the value (wildcards, or a plain substring)?
fn matches_any(value: &str, patterns: &[String]) -> bool {
    let haystack = value.to_lowercase();
    patterns.iter().any(|pattern| {
        let needle = pattern.trim().to_lowercase();
        !needle.is_empty() && (wildcard_match(&haystack, &needle) || haystack.contains(&needle))
    })
}

/// `*` for any run of characters, `?` for exactly one - the shell style, without
/// a regex engine, which is what the Python rules used as well (`fnmatch`).
fn wildcard_match(text: &str, pattern: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return text == pattern;
    }
    let text: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    // A two pointer walk with one remembered star: no exponential work, whatever
    // the text looks like.
    let (mut t, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            t += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A list of strings out of a JSON value (a single string counts as one item).
fn strings(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(Json::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Json::String(item)) => {
            let item = item.trim();
            if item.is_empty() {
                Vec::new()
            } else {
                vec![item.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

/// Where the label of a rule is written: the extra data of the record.
///
/// That is the same place the Python implementation uses (`extra.rule`), so a
/// marked notification stays marked for both programs and the details window
/// shows it - without a column of its own for something most records never have.
pub fn label_extra(decision: &Decision) -> Option<String> {
    let rule = decision.rule.as_ref()?;
    let mut object = serde_json::Map::new();
    object.insert("name".into(), Json::String(rule.clone()));
    if let Some(label) = &decision.label {
        object.insert("label".into(), Json::String(label.clone()));
    }
    Some(serde_json::to_string(&Json::Object(object)).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(aumid: &str, title: &str, body: &str) -> NotificationRecord {
        NotificationRecord {
            source: "wpndb".to_string(),
            aumid: Some(aumid.to_string()),
            title: Some(title.to_string()),
            body: Some(body.to_string()),
            ..Default::default()
        }
    }

    fn rule(name: &str, action: &str) -> Rule {
        Rule {
            name: name.to_string(),
            action: action.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn without_rules_everything_is_kept() {
        let set = RuleSet::default();
        let decision = set.evaluate(&record("Mail!App", "Счёт", "оплатите"));
        assert!(decision.store);
        assert!(!decision.alert);
        assert_eq!(decision.rule, None);
        assert_eq!(set.describe(), "правил нет");
    }

    #[test]
    fn a_noisy_application_can_be_skipped() {
        let mut noisy = rule("тишина", ACTION_SKIP);
        noisy.apps = vec!["steam*".to_string()];
        let set = RuleSet::new(vec![noisy]);
        let decision = set.evaluate(&record("Steam.Desktop", "Скидки", ""));
        assert!(decision.is_skipped(), "приложение из правила пропускается");
        assert_eq!(decision.rule.as_deref(), Some("тишина"));
        // Everything else is still stored.
        assert!(set.evaluate(&record("Mail!App", "Счёт", "")).store);
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let mut noisy = rule("тишина", ACTION_SKIP);
        noisy.apps = vec!["steam".to_string()];
        let mut important = rule("важное", ACTION_ALERT);
        important.text = vec!["скидки".to_string()];
        let set = RuleSet::new(vec![noisy, important]);
        // The skip rule comes first, so even a matching alert rule cannot save it.
        let decision = set.evaluate(&record("Steam", "Скидки", ""));
        assert!(decision.is_skipped());
        assert!(!decision.alert);
    }

    #[test]
    fn a_flag_rule_marks_the_record() {
        let mut flagged = rule("работа", ACTION_FLAG);
        flagged.text = vec!["зарплата".to_string()];
        let set = RuleSet::new(vec![flagged]);
        let decision = set.evaluate(&record("Mail!App", "Начисление", "Зарплата за сентябрь"));
        assert!(decision.store);
        assert_eq!(decision.label.as_deref(), Some("работа"));
        let extra = label_extra(&decision).expect("метка пишется в extra");
        assert!(extra.contains("\"name\":\"работа\""), "{extra}");
        assert!(extra.contains("\"label\":\"работа\""), "{extra}");
        // A record no rule matched carries nothing.
        let untouched = set.evaluate(&record("Mail!App", "Отпуск", ""));
        assert_eq!(label_extra(&untouched), None);
    }

    #[test]
    fn an_alert_rule_keeps_the_record_and_asks_for_a_pop_up() {
        let mut urgent = rule("срочно", ACTION_ALERT);
        urgent.text = vec!["срочно".to_string()];
        urgent.label = Some("внимание".to_string());
        let set = RuleSet::new(vec![urgent]);
        let decision = set.evaluate(&record("Discord", "СРОЧНО", "позвони"));
        assert!(decision.store, "оповещение не отменяет запись в журнал");
        assert!(decision.alert);
        assert_eq!(decision.label.as_deref(), Some("внимание"));

        let alert = Alert {
            seq: 7,
            application: "Discord".to_string(),
            title: "СРОЧНО".to_string(),
            body: "позвони".to_string(),
            label: decision.label.clone(),
        };
        assert_eq!(alert.heading(), "СРОЧНО");
        assert_eq!(alert.summary(), "Discord · позвони");
        let json = alert.to_json();
        assert_eq!(Alert::from_json(&json).as_ref(), Some(&alert));
        // An alert without a number cannot be told apart from an old one.
        assert_eq!(Alert::from_json(&serde_json::json!({"title": "x"})), None);
    }

    #[test]
    fn patterns_match_wildcards_substrings_and_any_case() {
        assert!(wildcard_match("steam.desktop", "steam*"));
        assert!(wildcard_match("steam.desktop", "*desktop"));
        assert!(wildcard_match("steam.desktop", "st??m*"));
        assert!(!wildcard_match("steam.desktop", "epic*"));
        assert!(wildcard_match("", "*"));
        assert!(wildcard_match("abc", "abc"));

        let patterns = vec!["STEAM".to_string()];
        assert!(matches_any("Steam.Desktop", &patterns), "регистр не важен");
        assert!(
            matches_any("Магазин Steam", &patterns),
            "подстрока тоже годится"
        );
        assert!(!matches_any("Epic", &patterns));
        assert!(
            !matches_any("Steam", &[String::new()]),
            "пустой шаблон не ловит всё"
        );
    }

    #[test]
    fn rules_can_select_by_kind_and_source() {
        let mut only_tiles = rule("плитки", ACTION_FLAG);
        only_tiles.types = vec!["tile".to_string()];
        only_tiles.sources = vec!["wpndb".to_string()];
        let set = RuleSet::new(vec![only_tiles]);

        let mut tile = record("Weather", "Погода", "");
        tile.notification_type = "tile".to_string();
        assert_eq!(set.evaluate(&tile).label.as_deref(), Some("плитки"));

        let toast = record("Weather", "Погода", "");
        assert_eq!(set.evaluate(&toast).rule, None, "тип не совпал");

        let mut other_source = record("Weather", "Погода", "");
        other_source.notification_type = "tile".to_string();
        other_source.source = "uns".to_string();
        assert_eq!(set.evaluate(&other_source).rule, None, "источник не совпал");
    }

    #[test]
    fn a_disabled_rule_is_ignored() {
        let mut disabled = rule("тишина", ACTION_SKIP);
        disabled.enabled = false;
        disabled.apps = vec!["*".to_string()];
        let set = RuleSet::new(vec![disabled]);
        assert!(set.evaluate(&record("Any", "x", "")).store);
        assert_eq!(set.enabled_count(), 0);
        assert_eq!(set.describe(), "все правила выключены");
    }

    #[test]
    fn rules_survive_a_round_trip_through_the_settings_file() {
        let mut noisy = rule("тишина", ACTION_SKIP);
        noisy.apps = vec!["Steam*".to_string(), "Epic".to_string()];
        noisy.text = vec!["скидки".to_string()];
        noisy.label = Some("шум".to_string());
        let mut urgent = rule("срочно", ACTION_ALERT);
        urgent.enabled = false;
        let set = RuleSet::new(vec![noisy, urgent]);

        let json = set.to_json();
        assert_eq!(RuleSet::from_json(&json), set);

        // The same list written by hand, in any spelling: one string instead of a
        // list, an action in capitals, an action nobody knows.
        let handwritten = serde_json::json!([
            {"name": "тишина", "apps": "Steam*", "action": "SKIP", "label": "шум"},
            {"name": "непонятное", "action": "что-то"},
        ]);
        let parsed = RuleSet::from_json(&handwritten);
        assert_eq!(parsed.rules().len(), 2);
        assert_eq!(parsed.rules()[0].action, ACTION_SKIP);
        assert_eq!(parsed.rules()[0].apps, vec!["Steam*".to_string()]);
        assert_eq!(
            parsed.rules()[1].action, ACTION_STORE,
            "неизвестное действие не ломает список"
        );
        assert_eq!(parsed.enabled_count(), 2);
        assert!(RuleSet::from_json(&serde_json::json!({"rules": []}))
            .rules()
            .is_empty());
    }

    #[test]
    fn the_rule_list_reads_in_the_log() {
        let mut noisy = rule("тишина", ACTION_SKIP);
        noisy.apps = vec!["Steam".to_string()];
        let set = RuleSet::new(vec![noisy, rule("важное", ACTION_FLAG)]);
        assert_eq!(set.describe(), "тишина:skip, важное:flag");
    }
}
