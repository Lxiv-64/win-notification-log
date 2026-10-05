//! The window that shows one notification in full.
//!
//! The list stays compact on purpose; anything that does not fit belongs here.
//! The window is opened per notification (double click, *Enter* or the button),
//! several of them can be open at once, and each one owns its data - closing it
//! closes nothing else.  Technical fields are available but hidden behind a
//! checkbox, so the normal case stays readable.

use crate::sqlite::Row;
use crate::win32::*;
use crate::{appicon, export, imaging, style, view};

const WINDOW_CLASS: &str = "WinNotificationLogDetail";
const WINDOW_TITLE: &str = "Уведомление";

const ID_ICON: usize = 401;
const ID_HEADING: usize = 402;
const ID_SUBHEADING: usize = 403;
const ID_BODY: usize = 404;
const ID_PREVIEW: usize = 405;
const ID_TABS: usize = 406;
const ID_FULL_TEXT: usize = 407;
const ID_COPY: usize = 408;
const ID_EXPORT: usize = 409;
const ID_CLOSE: usize = 410;

/// Margins and sizes in logical pixels (96 DPI).
const PADDING: i32 = style::PADDING;
const GAP: i32 = style::GAP;
const ICON_SIZE: i32 = 40;
const HEADING_HEIGHT: i32 = 26;
const SUBHEADING_HEIGHT: i32 = 20;
const BUTTON_HEIGHT: i32 = 28;
const BUTTON_WIDTH: i32 = 130;

/// Heading of the window: the title of the notification.
pub fn heading(row: &Row) -> String {
    match row.optional_text("title") {
        Some(title) => title,
        None => tr!("(без заголовка)").to_string(),
    }
}

/// Second line: who sent it, when, and how often it was seen.
pub fn subheading(row: &Row) -> String {
    let mut parts = vec![
        view::display_app(row),
        view::local_time(row.text("first_seen_utc").as_deref()),
    ];
    let seen = row.int("seen_count").unwrap_or(1);
    if seen > 1 {
        parts.push(tf!("повторов: {seen}", "repeats: {seen}"));
    }
    parts.join("  ·  ")
}

/// Everything the *Копировать* button puts on the clipboard.
pub fn clipboard_text(row: &Row) -> String {
    let mut lines = vec![heading(row), subheading(row), String::new()];
    let body = row.optional_text("body").unwrap_or_default();
    if !body.trim().is_empty() {
        lines.push(body);
        lines.push(String::new());
    }
    for (label, value) in view::readable_lines(row) {
        if label == tr!("Время")
            || label == tr!("Приложение")
            || label == tr!("Заголовок")
            || label == tr!("Текст")
        {
            continue;
        }
        lines.push(format!("{label}: {value}"));
    }
    lines.join("\r\n")
}

/// Text of the body area: message, extra lines, actions and image references.
pub fn body_text(row: &Row) -> String {
    let mut parts: Vec<String> = Vec::new();
    let text = row.optional_text("body").unwrap_or_default();
    if !text.trim().is_empty() {
        parts.push(text);
    }
    parts.extend(view::json_list(row.optional_text("extra_text")));
    let actions = view::actions(row);
    if !actions.is_empty() {
        parts.push(String::new());
        parts.push(tr!("Действия:").to_string());
        parts.extend(actions.into_iter().map(|action| format!("  • {action}")));
    }
    let images = view::images(row);
    if !images.is_empty() {
        parts.push(String::new());
        parts.push(tr!("Изображения:").to_string());
        parts.extend(images.into_iter().map(|image| format!("  • {image}")));
    }
    parts.join("\r\n")
}

/// Explanation of a picture that cannot be shown, if there is one.
///
/// A picture that *can* be shown wins over every reference that cannot: the
/// window either shows the first usable picture or explains why nothing is
/// there to show.
pub fn preview_note(row: &Row) -> Option<String> {
    let aumid = row.optional_text("aumid");
    let aumid = aumid.as_deref();
    let mut explanation: Option<String> = None;
    for image in view::images(row) {
        match imaging::availability(&image, aumid, preview_target()) {
            imaging::Availability::Local(_) => return None,
            ref other => {
                if explanation.is_none() {
                    explanation = Some(picture_note(&image, other));
                }
            }
        }
    }
    explanation
}

/// Target width for the picture of the details window, in device pixels.
fn preview_target() -> u32 {
    u32::try_from(imaging::MAX_PREVIEW_WIDTH).unwrap_or(320)
}

/// One line that says what stands in the way of one picture.
pub fn picture_note(reference: &str, availability: &imaging::Availability) -> String {
    match availability {
        imaging::Availability::Local(_) => String::new(),
        imaging::Availability::Remote(address) => tf!(
            "Картинка лежит в сети: {address}\n\
             Сама программа её не загружает (загрузка сообщила бы отправителю, \
             что уведомление открыли). «Открыть изображение» покажет её в браузере.",
            "The picture lives on the network: {address}\n\
             The program does not download it (a download would tell the sender \
             that the notification was opened). Use «Open the image» to see it in a browser."
        ),
        imaging::Availability::Missing if reference.starts_with("ms-appx") => tf!(
            "Картинка недоступна: {reference}\n\
             Файла нет среди файлов установленного приложения.",
            "The picture is unavailable: {reference}\n\
             The file is not among the files of the installed application."
        ),
        imaging::Availability::Missing => {
            tf!("Картинка недоступна: {reference}", "The picture is unavailable: {reference}")
        }
    }
}

/// Everything known about a notification: the page *Полностью* shows this.
///
/// It starts with the readable fields (so the page is useful on its own), then
/// adds the actions and pictures, and ends with the technical record: numbers,
/// keys, revisions and the payload.  Nothing is hidden and nothing is invented.
pub fn full_text(row: &Row) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut readable: Vec<String> = view::readable_lines(row)
        .into_iter()
        .map(|(label, value)| {
            if value.contains('\n') {
                format!("{label}:\r\n{value}")
            } else {
                format!("{label}: {value}")
            }
        })
        .collect();
    if let Some(note) = preview_note(row) {
        readable.push(note);
    }
    parts.push(tr!("Основные сведения").to_string());
    parts.push(String::new());
    parts.extend(readable);

    let actions = view::actions(row);
    if !actions.is_empty() {
        parts.push(String::new());
        parts.push(tr!("Действия уведомления").to_string());
        parts.push(String::new());
        parts.extend(actions.into_iter().map(|action| format!("  • {action}")));
    }
    let images = view::images(row);
    if !images.is_empty() {
        parts.push(String::new());
        parts.push(tr!("Изображения").to_string());
        parts.push(String::new());
        parts.extend(images.into_iter().map(|image| format!("  • {image}")));
    }
    let technical = view::technical_lines(row);
    if !technical.is_empty() {
        parts.push(String::new());
        parts.push(tr!("Технические подробности").to_string());
        parts.push(String::new());
        parts.extend(
            technical
                .into_iter()
                .map(|(label, value)| format!("{label}: {value}")),
        );
    }
    parts.join("\r\n")
}

/// State of one details window (owned through `GWLP_USERDATA`).
struct Detail {
    /// The window itself (set before the controls are created).
    hwnd: HWND,
    row: Row,
    body: String,
    full: String,
    image_note: Option<String>,
    icon: appicon::LoadedIcon,
    preview: Option<imaging::Preview>,
    font: HFONT,
    dpi: u32,
    /// Colours of the look in use (see `settings::Settings::theme`).
    palette: style::Palette,
    tab: HWND,
    icon_control: HWND,
    heading_control: HWND,
    subheading_control: HWND,
    body_control: HWND,
    preview_control: HWND,
    full_control: HWND,
    copy_button: HWND,
    export_button: HWND,
    close_button: HWND,
}

impl Detail {
    fn new(row: Row, dpi: u32, palette: style::Palette) -> Detail {
        let image_note = preview_note(&row);
        Detail {
            hwnd: std::ptr::null_mut(),
            body: body_text(&row),
            full: full_text(&row),
            image_note,
            icon: appicon::icon_for(
                row.optional_text("aumid").as_deref(),
                &Default::default(),
                px(ICON_SIZE, dpi),
            ),
            preview: None,
            row,
            font: system_font_scaled(dpi),
            dpi,
            tab: std::ptr::null_mut(),
            icon_control: std::ptr::null_mut(),
            heading_control: std::ptr::null_mut(),
            subheading_control: std::ptr::null_mut(),
            body_control: std::ptr::null_mut(),
            preview_control: std::ptr::null_mut(),
            full_control: std::ptr::null_mut(),
            copy_button: std::ptr::null_mut(),
            export_button: std::ptr::null_mut(),
            close_button: std::ptr::null_mut(),
            palette,
        }
    }

    /// The window font, applied to every control.
    fn owned_font(&self) -> HFONT {
        self.font
    }

    /// Which page is on top: 0 - the essentials, 1 - everything.
    fn page(&self) -> usize {
        if self.tab.is_null() {
            return 0;
        }
        tab_selected(self.tab)
    }

    /// Show the controls of the chosen page and hide the others.
    fn show_page(&self, page: usize) {
        unsafe {
            let first = if page == 0 { SW_SHOW } else { SW_HIDE };
            let second = if page == 1 { SW_SHOW } else { SW_HIDE };
            for control in [
                self.icon_control,
                self.heading_control,
                self.subheading_control,
                self.body_control,
                self.preview_control,
            ] {
                ShowWindow(control, first);
            }
            ShowWindow(self.full_control, second);
        }
    }
}

impl Drop for Detail {
    fn drop(&mut self) {
        if !self.font.is_null() {
            unsafe {
                DeleteObject(self.font as HGDIOBJ);
            }
        }
        // Only icons created for this window are freed; shared ones stay.
        self.icon.release();
    }
}

/// Scale a logical value to the monitor of this window.
fn px(value: i32, dpi: u32) -> i32 {
    scale(value, dpi)
}

/// Create the window for one notification.
///
/// Returns false when Windows refuses to create it (the caller shows a message).
pub fn open(parent: HWND, row: Row, dpi: u32, palette: style::Palette) -> bool {
    unsafe {
        register_class();
        let state = Box::new(Detail::new(row, dpi, palette));
        let raw = Box::into_raw(state);
        let title = wide(WINDOW_TITLE);
        let class = wide(WINDOW_CLASS);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(720, dpi.max(96)),
            px(640, dpi.max(96)),
            parent,
            std::ptr::null_mut(),
            module_handle(),
            raw as *mut std::ffi::c_void,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(raw));
            return false;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let state = &mut *raw;
        state.hwnd = hwnd;
        state.create_children(hwnd);
        state.show_page(0);
        state.layout_window();
        style::apply_look_to_window(
            hwnd,
            &state.palette,
            &[
                state.tab,
                state.icon_control,
                state.heading_control,
                state.subheading_control,
                state.body_control,
                state.preview_control,
                state.full_control,
                state.copy_button,
                state.export_button,
                state.close_button,
            ],
        );
        load_preview(state);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        true
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

impl Detail {
    /// Build every control of the window.
    fn create_children(&mut self, hwnd: HWND) {
        let instance = module_handle();
        let base = WS_CHILD | WS_VISIBLE;
        unsafe {
            self.icon_control =
                create_child(hwnd, instance, "STATIC", "", base | SS_ICON, 0, ID_ICON);
            self.heading_control = create_child(
                hwnd,
                instance,
                "STATIC",
                &heading(&self.row),
                base | style::LABEL,
                0,
                ID_HEADING,
            );
            self.subheading_control = create_child(
                hwnd,
                instance,
                "STATIC",
                &subheading(&self.row),
                base | style::LABEL | SS_ENDELLIPSIS,
                0,
                ID_SUBHEADING,
            );
            self.body_control = create_child(
                hwnd,
                instance,
                "EDIT",
                &self.body,
                base | WS_TABSTOP
                    | WS_BORDER
                    | WS_VSCROLL
                    | ES_MULTILINE
                    | ES_READONLY
                    | ES_AUTOVSCROLL,
                WS_EX_CLIENTEDGE,
                ID_BODY,
            );
            self.preview_control = create_child(
                hwnd,
                instance,
                "STATIC",
                "",
                base | SS_BITMAP,
                0,
                ID_PREVIEW,
            );
            self.full_control = create_child(
                hwnd,
                instance,
                "EDIT",
                &self.full,
                WS_CHILD | WS_VSCROLL | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL,
                WS_EX_CLIENTEDGE,
                ID_FULL_TEXT,
            );
            // Two pages: the essentials, and everything the record knows.
            self.tab = create_child(
                hwnd,
                instance,
                TAB_CLASS,
                "",
                base | WS_TABSTOP | TCS_MULTILINE,
                0,
                ID_TABS,
            );
            tab_insert(self.tab, "Основное");
            tab_insert(self.tab, "Полностью");
            // The tab control is the frame: it stays behind the page contents,
            // which are separate siblings placed inside the display area.
            set_z_order(self.tab, HWND_BOTTOM);
            self.copy_button = create_child(
                hwnd,
                instance,
                "BUTTON",
                "Копировать",
                base | WS_TABSTOP | BS_PUSHBUTTON,
                0,
                ID_COPY,
            );
            self.export_button = create_child(
                hwnd,
                instance,
                "BUTTON",
                "Экспорт…",
                base | WS_TABSTOP | BS_PUSHBUTTON,
                0,
                ID_EXPORT,
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
            // The system font everywhere: the window has to match the system.
            let font = self.owned_font();
            for control in [
                self.icon_control,
                self.heading_control,
                self.subheading_control,
                self.body_control,
                self.full_control,
                self.tab,
                self.copy_button,
                self.export_button,
                self.close_button,
            ] {
                SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
            }
            // The heading is the one line that has to stand out.
            SendMessageW(
                self.heading_control,
                WM_SETFONT,
                bold_font_scaled(self.dpi) as WPARAM,
                1,
            );
            if !self.icon.handle.is_null() {
                SendMessageW(
                    self.icon_control,
                    STM_SETIMAGE,
                    IMAGE_ICON,
                    self.icon.handle as LPARAM,
                );
            }
        }
    }

    /// Place the controls: header (icon, title, source), body, picture, buttons.
    fn layout_window(&self) {
        let mut client = RECT::default();
        unsafe { GetClientRect(self.window(), &mut client) };
        let width = client.width();
        let height = client.height();
        let dpi = self.dpi;
        let padding = px(PADDING, dpi);
        let gap = px(GAP, dpi);
        let icon = px(ICON_SIZE, dpi);
        let heading_height = px(HEADING_HEIGHT, dpi);
        let subheading_height = px(SUBHEADING_HEIGHT, dpi);
        let button_height = px(BUTTON_HEIGHT, dpi);
        let button_width = px(BUTTON_WIDTH, dpi);

        // Buttons along the bottom; the pages live above them.
        let buttons_top = height - padding - button_height;
        let mut right = width - padding;
        for (control, width) in [
            (self.close_button, button_width),
            (self.export_button, button_width),
            (self.copy_button, button_width),
        ] {
            right -= width;
            unsafe { MoveWindow(control, right, buttons_top, width, button_height, 1) };
            right -= gap;
        }

        // The tab control fills the whole content area; `TCM_ADJUSTRECT` tells
        // us where the page itself starts, so the frame is drawn by the control.
        let frame_left = padding;
        let frame_top = padding;
        let frame_width = (width - 2 * padding).max(px(240, dpi));
        let frame_height = (buttons_top - gap - padding).max(px(160, dpi));
        unsafe {
            MoveWindow(
                self.tab,
                frame_left,
                frame_top,
                frame_width,
                frame_height,
                1,
            );
        }
        // The tab control fills the whole content area; its strip answers for its
        // own height, so the page starts right below the tabs.  `TCM_ADJUSTRECT`
        // is not used: Windows gives its two directions an opposite meaning than
        // the documentation suggests, and measuring the tab is exact anyway.
        let strip = tab_strip_height(self.tab, px(26, dpi));
        let border = px(3, dpi);
        let page = RECT {
            left: frame_left + border,
            top: frame_top + strip,
            right: frame_left + frame_width - border,
            bottom: frame_top + frame_height - border,
        };
        let page_width = page.width().max(px(120, dpi));
        let page_height = page.height().max(px(80, dpi));

        // Page 1: everything in one read-only field.
        unsafe {
            MoveWindow(
                self.full_control,
                page.left,
                page.top,
                page_width,
                page_height,
                1,
            );
        }

        // Page 0: header, body, picture.
        let mut top = page.top;
        unsafe {
            MoveWindow(self.icon_control, page.left, top, icon, icon, 1);
            MoveWindow(
                self.heading_control,
                page.left + icon + gap,
                top,
                (page_width - icon - gap).max(80),
                heading_height,
                1,
            );
            MoveWindow(
                self.subheading_control,
                page.left + icon + gap,
                top + heading_height,
                (page_width - icon - gap).max(80),
                subheading_height,
                1,
            );
        }
        top += icon.max(heading_height + subheading_height) + gap;

        let mut body_bottom = page.top + page_height;
        let preview_height = self
            .preview
            .as_ref()
            .map(|preview| preview.height)
            .unwrap_or(0);
        if preview_height > 0 {
            let preview_width = self
                .preview
                .as_ref()
                .map(|preview| preview.width)
                .unwrap_or(0);
            unsafe {
                MoveWindow(
                    self.preview_control,
                    page.left,
                    body_bottom - preview_height,
                    preview_width,
                    preview_height,
                    1,
                );
            }
            body_bottom -= preview_height + gap;
        }

        // The note about an unavailable picture is shown inside the body text
        // (see `body_text`), so the preview only needs the space it really uses.
        let body_height = (body_bottom - top).max(px(80, dpi));
        unsafe {
            MoveWindow(
                self.body_control,
                page.left,
                top,
                page_width,
                body_height,
                1,
            );
        }
    }

    fn window(&self) -> HWND {
        self.hwnd
    }
}

/// Create one child control with the window font applied.
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

/// Decode the first picture of the notification that can be shown.
///
/// The reference may be a path, a file inside an application package or a web
/// address; only the first two can be shown, and the package variants are chosen
/// for the size of the preview area.
fn load_preview(state: &mut Detail) {
    let dpi = state.dpi;
    let width = px(imaging::MAX_PREVIEW_WIDTH, dpi);
    let height = px(imaging::MAX_PREVIEW_HEIGHT, dpi);
    let target = u32::try_from(width).unwrap_or(imaging::ANY_SIZE);
    let aumid = state.row.optional_text("aumid");
    let Some(path) = view::images(&state.row)
        .iter()
        .find_map(|image| imaging::resolve(image, aumid.as_deref(), target))
    else {
        return;
    };
    let Some(preview) = imaging::load(&path, width, height) else {
        return;
    };
    unsafe {
        SendMessageW(
            state.preview_control,
            STM_SETIMAGE,
            IMAGE_BITMAP,
            preview.bitmap as LPARAM,
        );
    }
    state.preview = Some(preview);
}

/// State of a window from its `GWLP_USERDATA`.
unsafe fn state_of(hwnd: HWND) -> Option<&'static mut Detail> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Detail;
    raw.as_mut()
}

/// Window procedure of a notification window.
unsafe extern "system" fn window_proc(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_SIZE => {
            if let Some(state) = state_of(hwnd) {
                state.layout_window();
            }
            0
        }
        WM_COMMAND => {
            let id = (w & 0xFFFF) as usize;
            if let Some(state) = state_of(hwnd) {
                match id {
                    ID_COPY => {
                        copy_to_clipboard(hwnd, &clipboard_text(&state.row));
                    }
                    ID_EXPORT => export_one(state, hwnd),
                    ID_CLOSE => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_NOTIFY => {
            // The tab control tells us when the user switched pages.
            let header = notify_header(l);
            if header.code == TCN_SELCHANGE {
                if let Some(state) = state_of(hwnd) {
                    let page = state.page();
                    state.show_page(page);
                    state.layout_window();
                }
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            // Every label is painted on the window background, otherwise each of
            // them carries a grey rectangle; the second line of the heading is
            // the one note in this window, so it is drawn in the grey for text.
            let control = l as HWND;
            let Some(state) = state_of(hwnd) else {
                return DefWindowProcW(hwnd, message, w, l);
            };
            let muted = control == state.subheading_control;
            match style::paint_control(w as HDC, control, &state.palette, muted) {
                Some(background) => background as LRESULT,
                None => DefWindowProcW(hwnd, message, w, l),
            }
        }
        WM_ERASEBKGND => match state_of(hwnd) {
            Some(state) => style::erase_background(hwnd, w as HDC, &state.palette),
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => match state_of(hwnd) {
            // The two read-only bodies and the buttons: without this they would be
            // white boxes on a dark window.
            Some(state) => style::paint_surface(message, w as HDC, &state.palette) as LRESULT,
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Detail;
            if !raw.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

/// Save this one notification in the format the user picks.
fn export_one(state: &Detail, hwnd: HWND) {
    let format = export::Format::Csv;
    let stamp = view::local_time(Some(&crate::time::now_iso()));
    let suggested = format!("notification-{}.{}", now_stamp(), format.extension());
    let rows = vec![state.row.clone()];
    match export::save(hwnd, &rows, format, &suggested, &stamp) {
        Ok(path) => {
            let text = tf!("Файл сохранён:\r\n{}", "File saved:\r\n{}", path.display());
            set_window_text(state.body_control, &format!("{text}\r\n\r\n{}", state.body));
        }
        Err(error) => message_box(hwnd, &error, MB_ICONINFORMATION),
    }
}

/// `YYYYMMDD-HHMMSS` of the current local time.
fn now_stamp() -> String {
    let digits: String = view::local_time(Some(&crate::time::now_iso()))
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if digits.len() >= 14 {
        format!(
            "{}{}{}-{}",
            &digits[4..8],
            &digits[2..4],
            &digits[0..2],
            &digits[8..14]
        )
    } else {
        "export".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{Row, Value};

    fn sample(pairs: &[(&str, &str)]) -> Row {
        let names: Vec<String> = pairs.iter().map(|(key, _)| key.to_string()).collect();
        let values: Vec<Value> = pairs
            .iter()
            .map(|(_, value)| Value::Text(value.to_string()))
            .collect();
        Row::from_pairs(names, values)
    }

    #[test]
    fn heading_falls_back_when_there_is_no_title() {
        assert_eq!(heading(&sample(&[("title", "Письмо")])), "Письмо");
        assert_eq!(heading(&sample(&[("body", "текст")])), "(без заголовка)");
    }

    #[test]
    fn subheading_names_the_application_time_and_repeats() {
        let row = sample(&[
            ("app_name", "Почта"),
            ("first_seen_utc", "2026-09-29T12:00:00.000000Z"),
            ("seen_count", "3"),
        ]);
        let text = subheading(&row);
        assert!(text.starts_with("Почта"));
        assert!(text.contains('·'));
        assert!(text.contains("повторов: 3"));

        let once = sample(&[("app_name", "Chat"), ("seen_count", "1")]);
        assert!(!subheading(&once).contains("повторов"));
    }

    #[test]
    fn clipboard_text_carries_everything_readable() {
        let row = sample(&[
            ("title", "Письмо"),
            ("app_name", "Почта"),
            ("first_seen_utc", "2026-09-29T12:00:00.000000Z"),
            ("body", "Текст письма"),
            ("launch", "https://example.org/mail"),
        ]);
        let text = clipboard_text(&row);
        assert!(text.starts_with("Письмо\r\n"));
        assert!(text.contains("Текст письма"));
        assert!(text.contains("Ссылка: https://example.org/mail"));
        assert!(
            !text.contains("Заголовок: "),
            "the heading is already the first line"
        );
    }

    #[test]
    fn body_lists_actions_and_images() {
        let row = sample(&[
            ("body", "Основной текст"),
            ("actions", r#"[{"content":"Открыть","type":"foreground"}]"#),
            ("images", r#"["https://example.org/a.png"]"#),
        ]);
        let text = body_text(&row);
        assert!(text.starts_with("Основной текст"));
        assert!(text.contains("Действия:"));
        assert!(text.contains("• “Открыть” [foreground]"));
        assert!(text.contains("Изображения:"));
        assert_eq!(
            picture_note(
                "https://example.org/a.png",
                &imaging::Availability::Remote("https://example.org/a.png".to_string())
            ),
            preview_note(&row).expect("a web address has no picture to show")
        );
        assert!(
            preview_note(&row)
                .expect("the web address cannot be shown")
                .contains("https://example.org/a.png"),
            "the explanation names the address"
        );

        // A package picture of an application that is not installed is missing,
        // and the explanation says so instead of talking about the network.
        let package = sample(&[
            ("aumid", "Definitely.Not.Installed_x!App"),
            ("images", r#"["ms-appx:///Assets/Logo.png"]"#),
        ]);
        let note = preview_note(&package).expect("the file is not there");
        assert!(
            note.contains("нет среди файлов установленного приложения"),
            "unexpected explanation: {note}"
        );

        // Without a picture there is nothing to explain.
        assert_eq!(preview_note(&sample(&[("body", "текст")])), None);
    }

    #[test]
    fn the_full_page_holds_everything_the_record_knows() {
        let row = sample(&[
            ("title", "Письмо"),
            ("app_name", "Почта"),
            ("first_seen_utc", "2026-09-29T12:00:00.000000Z"),
            ("body", "Текст письма"),
            ("launch", "https://example.org/mail"),
            ("notification_type", "toast"),
            ("sources", "wpndb"),
            ("seen_count", "3"),
            ("dedup_key", "key-1"),
            ("revision", "2"),
            ("id", "17"),
        ]);
        let text = full_text(&row);
        for expected in [
            "Основные сведения",
            "Текст письма",
            "https://example.org/mail",
            "Технические подробности",
            "Ключ: key-1",
            "ревизия 2",
        ] {
            assert!(
                text.contains(expected),
                "«{expected}» missing from:\n{text}"
            );
        }
        assert!(
            text.len() > body_text(&row).len(),
            "the full page shows more than the body"
        );
        assert!(
            text.find("Основные сведения").unwrap() < text.find("Технические подробности").unwrap(),
            "readable fields come first, technical ones after"
        );
    }

    #[test]
    fn the_full_page_never_claims_what_is_not_there() {
        let row = sample(&[("body", "только текст")]);
        let text = full_text(&row);
        assert!(text.contains("только текст"));
        assert!(
            !text.contains("Действия уведомления"),
            "no actions means no heading for them"
        );
        assert!(!text.contains("Изображения"));
    }

    #[test]
    fn the_full_page_lists_what_cannot_be_previewed() {
        let row = sample(&[
            ("body", "текст"),
            ("images", r#"["https://example.org/a.png"]"#),
        ]);
        let text = full_text(&row);
        assert!(
            text.contains("Изображения"),
            "the pictures get their own section:\n{text}"
        );
        assert!(text.contains("https://example.org/a.png"), "{text}");
        assert!(
            text.contains("Картинка лежит в сети"),
            "the page explains why there is no picture:\n{text}"
        );
        assert!(
            text.contains("не загружает"),
            "and it explains that the program does not fetch it:\n{text}"
        );
    }
}
