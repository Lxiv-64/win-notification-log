//! The window that deletes the journal for a chosen period.
//!
//! Deleting by hand (one record, one selection) lives in the list; a *range* of
//! dates needs a place where both bounds can be typed and read back, and where
//! the user is told how many records are about to disappear.  That is this window.
//!
//! The window owns no data: it collects two dates and asks its owner (the main
//! window) two questions - "сколько записей попадает в этот период" and, when the
//! user insists, "удали их".  All journal access stays in one place, and the two
//! questions are ordinary messages inside one process, which keeps the split
//! honest (see the README on cross-process messages).

use crate::view;
use crate::win32::*;
use crate::style;

const WINDOW_CLASS: &str = "WinNotificationLogPurge";
const WINDOW_TITLE: &str = "Удаление за период";

const ID_HINT: usize = 601;
const ID_FROM_LABEL: usize = 602;
const ID_FROM: usize = 603;
const ID_TO_LABEL: usize = 604;
const ID_TO: usize = 605;
const ID_SUMMARY: usize = 606;
const ID_DELETE: usize = 607;
const ID_CLOSE: usize = 608;

/// Margins and row heights in logical pixels (96 DPI).
const PADDING: i32 = style::PADDING;
const GAP: i32 = style::GAP;
const HINT_HEIGHT: i32 = 46;
const FIELD_HEIGHT: i32 = style::ROW;
const SUMMARY_HEIGHT: i32 = 52;
const BUTTON_HEIGHT: i32 = style::ROW;
const BUTTON_WIDTH: i32 = 140;
const CLOSE_WIDTH: i32 = 110;
const FIELD_WIDTH: i32 = 110;
const FROM_LABEL_WIDTH: i32 = 16;
const TO_LABEL_WIDTH: i32 = 26;
const SMALL_GAP: i32 = 6;
const WINDOW_WIDTH: i32 = 560;
const WINDOW_HEIGHT: i32 = 214;
/// The count is asked after a short pause, so typing does not query per keystroke.
const TIMER_SUMMARY: usize = 1;
const SUMMARY_DELAY_MS: u32 = 250;

/// Message of the owner: "count the period in this dialog" (returns the count).
pub const MSG_COUNT: UINT = WM_APP + 20;
/// Message of the owner: "delete the period in this dialog" (returns rows deleted).
pub const MSG_DELETE: UINT = WM_APP + 21;

/// What the dialog tells the user about the typed range.
///
/// Pure, so the wording and the four cases (nothing typed, an unreadable date, a
/// half filled range, both dates) can be checked without a window.  `count` is
/// what the owner answered; `-1` means "the count could not be worked out".
pub fn summary_text(from: &str, to: &str, count: i64) -> String {
    let typed_from = from.trim();
    let typed_to = to.trim();
    let (since, until) = view::custom_range(typed_from, typed_to);
    if since.is_none() && until.is_none() {
        return if typed_from.is_empty() && typed_to.is_empty() {
            tr!("Укажите хотя бы одну дату: без границ будет удалён весь журнал.").to_string()
        } else {
            tr!("Дату не удалось прочитать. Примеры: 29.09.2026, 29.09.26, 2026-09-29.")
                .to_string()
        };
    }
    let what = match (&since, &until) {
        (Some(_), Some(_)) => tf!(
            "записи с {typed_from} по {typed_to} (обе даты включительно)",
            "records from {typed_from} to {typed_to} (both dates inclusive)"
        ),
        (Some(_), None) => tf!(
            "записи с {typed_from} и новее",
            "records from {typed_from} and newer"
        ),
        _ => tf!("записи по {typed_to} и старше", "records up to {typed_to} and older"),
    };
    match count {
        0 => tf!(
            "Под удаление ничего не попадает: {what} не найдены.",
            "Nothing would be deleted: no {what} were found."
        ),
        value if value > 0 => tf!(
            "Будет удалено записей: {value} — {what}.",
            "Records to delete: {value} — {what}."
        ),
        _ => tf!(
            "Под удаление попадут {what}; подсчитать не удалось.",
            "{what} would be deleted; the count could not be worked out."
        ),
    }
}

/// Is the typed range complete enough to be deleted?
fn ready(from: &str, to: &str) -> bool {
    let (since, until) = view::custom_range(from.trim(), to.trim());
    since.is_some() || until.is_some()
}

/// The texts of the two date fields of `dialog`.
pub fn bounds(dialog: HWND) -> (String, String) {
    unsafe {
        (
            window_text(GetDlgItem(dialog, ID_FROM as i32)),
            window_text(GetDlgItem(dialog, ID_TO as i32)),
        )
    }
}

/// State of the window; owned by the window (`GWLP_USERDATA`).
struct Purge {
    hwnd: HWND,
    /// The main window: it owns the journal and answers both questions.
    owner: HWND,
    /// Monitor DPI combined with the text size of the main window.
    dpi: u32,
    font: HFONT,
    /// Colours of the look in use (see `settings::Settings::theme`).
    palette: style::Palette,
    hint: HWND,
    from_label: HWND,
    from: HWND,
    to_label: HWND,
    to: HWND,
    summary: HWND,
    delete_button: HWND,
    close_button: HWND,
}

/// Logical pixel to monitor pixel.
fn px(value: i32, dpi: u32) -> i32 {
    scale(value, dpi)
}

/// Create one child control.
fn create_child(
    parent: HWND,
    instance: HINSTANCE,
    class: &str,
    text: &str,
    style: UINT,
    ex_style: UINT,
    id: usize,
) -> HWND {
    let class_w = wide(class);
    let text_w = wide(text);
    unsafe {
        CreateWindowExW(
            ex_style,
            class_w.as_ptr(),
            text_w.as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            instance,
            std::ptr::null_mut(),
        )
    }
}

impl Purge {
    /// Every control of the window (for the font and for the look).
    fn controls(&self) -> Vec<HWND> {
        vec![
            self.hint,
            self.from_label,
            self.from,
            self.to_label,
            self.to,
            self.summary,
            self.delete_button,
            self.close_button,
        ]
    }

    fn new(owner: HWND, dpi: u32, palette: style::Palette) -> Purge {
        Purge {
            hwnd: std::ptr::null_mut(),
            owner,
            dpi,
            font: system_font_scaled(dpi),
            palette,
            hint: std::ptr::null_mut(),
            from_label: std::ptr::null_mut(),
            from: std::ptr::null_mut(),
            to_label: std::ptr::null_mut(),
            to: std::ptr::null_mut(),
            summary: std::ptr::null_mut(),
            delete_button: std::ptr::null_mut(),
            close_button: std::ptr::null_mut(),
        }
    }

    /// Build every control of the window.
    fn create_children(&mut self, hwnd: HWND, from: &str, to: &str) {
        let instance = module_handle();
        let base = WS_CHILD | WS_VISIBLE;
        unsafe {
            self.hint = create_child(
                hwnd,
                instance,
                "STATIC",
                "Удаляются записи журнала за выбранные сутки (местное время). \
                 Даты: 29.09.2026, 29.09.26, 2026-09-29. Пустое поле — граница не задана.",
                base | style::HINT,
                0,
                ID_HINT,
            );
            self.from_label = create_child(
                hwnd,
                instance,
                "STATIC",
                "с",
                base | style::LABEL,
                0,
                ID_FROM_LABEL,
            );
            self.from = create_child(
                hwnd,
                instance,
                "EDIT",
                from,
                base | WS_TABSTOP | ES_LEFT | ES_AUTOHSCROLL,
                WS_EX_CLIENTEDGE,
                ID_FROM,
            );
            self.to_label = create_child(
                hwnd,
                instance,
                "STATIC",
                "по",
                base | style::LABEL,
                0,
                ID_TO_LABEL,
            );
            self.to = create_child(
                hwnd,
                instance,
                "EDIT",
                to,
                base | WS_TABSTOP | ES_LEFT | ES_AUTOHSCROLL,
                WS_EX_CLIENTEDGE,
                ID_TO,
            );
            self.summary = create_child(
                hwnd,
                instance,
                "STATIC",
                "",
                base | style::LABEL,
                0,
                ID_SUMMARY,
            );
            self.delete_button = create_child(
                hwnd,
                instance,
                "BUTTON",
                "Удалить…",
                base | WS_TABSTOP | BS_PUSHBUTTON,
                0,
                ID_DELETE,
            );
            self.close_button = create_child(
                hwnd,
                instance,
                "BUTTON",
                "Закрыть",
                base | WS_TABSTOP | BS_PUSHBUTTON,
                0,
                ID_CLOSE,
            );
            for control in [
                self.hint,
                self.from_label,
                self.from,
                self.to_label,
                self.to,
                self.summary,
                self.delete_button,
                self.close_button,
            ] {
                SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
            }
            // The line with the count is the one that matters here.
            SendMessageW(
                self.summary,
                WM_SETFONT,
                bold_font_scaled(self.dpi) as WPARAM,
                1,
            );
        }
    }

    /// Place the controls: hint, two dates, the count, the buttons.
    fn layout(&self) {
        let mut client = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let width = client.width();
        let height = client.height();
        let dpi = self.dpi;
        let padding = px(PADDING, dpi);
        let gap = px(GAP, dpi);
        let field_height = px(FIELD_HEIGHT, dpi);
        let button_height = px(BUTTON_HEIGHT, dpi);
        let field_width = px(FIELD_WIDTH, dpi);

        let mut top = padding;
        unsafe {
            MoveWindow(
                self.hint,
                padding,
                top,
                (width - 2 * padding).max(px(200, dpi)),
                px(HINT_HEIGHT, dpi),
                1,
            );
        }
        top += px(HINT_HEIGHT, dpi) + gap;

        // The two date fields, one after another; the labels sit on their line.
        let mut left = padding;
        for (control, control_width, label) in [
            (self.from_label, px(FROM_LABEL_WIDTH, dpi), true),
            (self.from, field_width, false),
            (self.to_label, px(TO_LABEL_WIDTH, dpi), true),
            (self.to, field_width, false),
        ] {
            let y = if label { top + px(4, dpi) } else { top };
            unsafe { MoveWindow(control, left, y, control_width, field_height, 1) };
            left += control_width + px(SMALL_GAP, dpi);
        }
        top += field_height + gap;

        unsafe {
            MoveWindow(
                self.summary,
                padding,
                top,
                (width - 2 * padding).max(px(200, dpi)),
                px(SUMMARY_HEIGHT, dpi),
                1,
            );
        }

        // Buttons along the bottom, right aligned, like every Windows dialog.
        let mut right = width - padding;
        let buttons_top = height - padding - button_height;
        for (control, control_width) in [
            (self.close_button, px(CLOSE_WIDTH, dpi)),
            (self.delete_button, px(BUTTON_WIDTH, dpi)),
        ] {
            right -= control_width;
            unsafe { MoveWindow(control, right, buttons_top, control_width, button_height, 1) };
            right -= gap;
        }
    }

    /// Read both fields, refresh the count and the state of the delete button.
    fn refresh(&self) {
        let (from, to) = bounds(self.hwnd);
        let usable = ready(&from, &to) && !self.owner.is_null();
        let count = if usable {
            // The main window counts (and deletes); this window only asks.
            unsafe { SendMessageW(self.owner, MSG_COUNT, self.hwnd as WPARAM, 0) as i64 }
        } else {
            -1
        };
        set_window_text(self.summary, &summary_text(&from, &to, count));
        unsafe {
            EnableWindow(self.delete_button, if usable { 1 } else { 0 });
        }
    }
}

impl Drop for Purge {
    fn drop(&mut self) {
        if !self.font.is_null() {
            unsafe {
                DeleteObject(self.font as HGDIOBJ);
            }
        }
    }
}

/// State of a window from its `GWLP_USERDATA`.
unsafe fn state_of(hwnd: HWND) -> Option<&'static mut Purge> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Purge;
    raw.as_mut()
}

/// Give the focus to the next (or previous) control, the way Tab does.
fn focus_next(dialog: HWND, previous: bool) {
    unsafe {
        let next = GetNextDlgTabItem(dialog, GetFocus(), if previous { 1 } else { 0 });
        if !next.is_null() {
            SetFocus(next);
        }
    }
}

/// Window procedure of the period window.
unsafe extern "system" fn window_proc(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        // Labels keep no background of their own: without this every caption of
        // the window paints a grey rectangle on the white background.
        WM_ERASEBKGND => match state_of(hwnd) {
            Some(state) => style::erase_background(hwnd, w as HDC, &state.palette),
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CTLCOLORSTATIC => {
            let control = l as HWND;
            match state_of(hwnd).map(|state| state.palette) {
                Some(palette) => {
                    let muted = GetDlgCtrlID(control) as usize == ID_HINT;
                    match style::paint_control(w as HDC, control, &palette, muted) {
                        Some(background) => background as LRESULT,
                        None => DefWindowProcW(hwnd, message, w, l),
                    }
                }
                None => DefWindowProcW(hwnd, message, w, l),
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => match state_of(hwnd) {
            // The date fields and the two buttons: in the night look they would
            // otherwise be white boxes on a dark window.
            Some(state) => style::paint_surface(message, w as HDC, &state.palette) as LRESULT,
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_SIZE => {
            if let Some(state) = state_of(hwnd) {
                state.layout();
            }
            0
        }
        WM_COMMAND => {
            let id = (w & 0xFFFF) as usize;
            let code = ((w >> 16) & 0xFFFF) as UINT;
            if (id == ID_FROM || id == ID_TO) && code == EN_CHANGE {
                // Typing is debounced; the count follows the field.
                SetTimer(hwnd, TIMER_SUMMARY, SUMMARY_DELAY_MS, std::ptr::null_mut());
                return 0;
            }
            if let Some(state) = state_of(hwnd) {
                match id {
                    ID_DELETE => {
                        // The owner counts, asks the user and deletes; anything
                        // but a positive answer means "nothing was deleted".
                        let owner = state.owner;
                        EnableWindow(hwnd, 0);
                        let deleted = SendMessageW(owner, MSG_DELETE, hwnd as WPARAM, 0) as i64;
                        EnableWindow(hwnd, 1);
                        if deleted > 0 {
                            DestroyWindow(hwnd);
                        } else {
                            state.refresh();
                        }
                    }
                    ID_CLOSE => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_TIMER => {
            if w == TIMER_SUMMARY {
                KillTimer(hwnd, TIMER_SUMMARY);
                if let Some(state) = state_of(hwnd) {
                    state.refresh();
                }
            }
            0
        }
        WM_KEYDOWN => match w as UINT {
            // This window is not a dialog resource, so Tab is moved by hand.
            VK_TAB => {
                focus_next(hwnd, GetKeyState(VK_SHIFT) < 0);
                0
            }
            VK_ESCAPE => {
                DestroyWindow(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Purge;
            if !raw.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

/// Register the window class once.
fn register_class() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| unsafe {
        let class_name = wide(WINDOW_CLASS);
        let class = WNDCLASSEXW {
            lpfn_wnd_proc: Some(window_proc),
            h_instance: module_handle(),
            h_icon: crate::appicon::application_icon(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbr_background: COLOR_WINDOW_BRUSH,
            lpsz_class_name: class_name.as_ptr(),
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

/// Open the period window; the filter dates are offered as a starting point.
pub fn open(parent: HWND, dpi: u32, from: &str, to: &str, palette: style::Palette) -> bool {
    register_class();
    let raw = Box::into_raw(Box::new(Purge::new(parent, dpi, palette)));
    unsafe {
        let title = wide(WINDOW_TITLE);
        let class = wide(WINDOW_CLASS);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(WINDOW_WIDTH, dpi.max(96)),
            px(WINDOW_HEIGHT, dpi.max(96)),
            parent,
            std::ptr::null_mut(),
            module_handle(),
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            drop(Box::from_raw(raw));
            return false;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let state = &mut *raw;
        state.hwnd = hwnd;
        state.create_children(hwnd, from.trim(), to.trim());
        state.layout();
        style::apply_look_to_window(hwnd, &state.palette, &state.controls());
        state.refresh();
        SetFocus(GetDlgItem(hwnd, ID_FROM as i32));
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_line_names_the_period() {
        // Both dates: the message says which days go and how many records.
        let both = summary_text("29.09.2026", "30.09.2026", 428);
        assert!(both.contains("записей: 428"), "{both}");
        assert!(both.contains("29.09.2026"), "{both}");
        assert!(both.contains("30.09.2026"), "{both}");

        // One date is enough - the other bound simply does not exist.
        assert!(summary_text("29.09.2026", "", 3).contains("и новее"));
        assert!(summary_text("", "30.09.2026", 3).contains("и старше"));

        // An unreadable date, and the dangerous empty range, are named as such.
        assert!(summary_text("вчера", "", 0).contains("не удалось прочитать"));
        assert!(summary_text("", "", 0).contains("весь журнал"));

        // A count of zero is good news, not a deletion.
        assert!(summary_text("29.09.2026", "", 0).contains("ничего не попадает"));
        // An unknown count (the journal refused) still explains the period.
        assert!(summary_text("29.09.2026", "", -1).contains("подсчитать не удалось"));
    }

    #[test]
    fn only_a_readable_range_can_be_deleted() {
        assert!(!ready("", ""));
        assert!(!ready("вчера", ""));
        assert!(ready("29.09.2026", ""));
        assert!(ready("", "29.09.2026"));
        assert!(ready("29.09.2026", "30.09.2026"));
        // A readable start with an unreadable end is still a lower bound.
        assert!(ready("29.09.2026", "завтра"));
    }
}

