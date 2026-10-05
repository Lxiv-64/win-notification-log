//! One visual language for every window of the application.
//!
//! The four windows used to invent their own margins (8/6, 12/8, 14/8, 16/8),
//! their own row heights (24/26/30) and their own idea of how a label sits next
//! to its field - one of them even moved the label down by six pixels by hand.
//! Two defects came out of that and both were visible in every window:
//!
//! * a static control keeps the button-face background unless the window answers
//!   `WM_CTLCOLORSTATIC`, so every label painted a grey rectangle on the white
//!   window: a fill under the text that lines up with nothing;
//! * a label that is placed by eye drifts as soon as the text size changes.
//!
//! Here live the tokens (one spacing scale, one set of heights) and the small
//! helpers that place a label on exactly the same line as the field next to it
//! and paint it as part of the window.

use crate::win32::*;

// ------------------------------------------------------------------- spacing

/// Outer margin of every window.
pub const PADDING: i32 = 14;
/// Distance between two neighbours in one row.
pub const GAP: i32 = 8;
/// Distance between two rows.
pub const ROW_GAP: i32 = 10;
/// Distance between two sections of a dialog.
pub const SECTION_GAP: i32 = 18;
/// Height of an input field, a combo box or a button.
pub const ROW: i32 = 28;
/// Height of one line of plain text.
pub const LINE: i32 = 18;
/// Height of a section heading.
pub const HEADING: i32 = 22;

// -------------------------------------------------------------------- styles

/// A label that shares its line with the field beside it.
///
/// `SS_CENTERIMAGE` does the vertical centring, so the caller only has to give
/// the label the box of the field: no arithmetic that can drift by a pixel.
/// `SS_ENDELLIPSIS` keeps a caption that is too long readable - it ends in an
/// ellipsis instead of being cut in the middle of a word under the next control.
pub const LABEL: UINT = SS_LEFT | SS_CENTERIMAGE | SS_NOPREFIX | SS_ENDELLIPSIS;
/// Secondary text: same line, but read as a note rather than as a caption.
pub const HINT: UINT = SS_LEFT | SS_NOPREFIX;
/// Section heading of a dialog.
pub const TITLE: UINT = SS_LEFT | SS_CENTERIMAGE | SS_NOPREFIX;

/// A single line text field.
///
/// Windows draws two different frames for `WS_BORDER` and `WS_EX_CLIENTEDGE`;
/// asking for both (as every field here used to) leaves a doubled edge.
pub const EDIT: UINT = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL;
pub const EDIT_EX: UINT = WS_EX_CLIENTEDGE;
/// A drop down list: no free text, the list needs room below the field.
pub const COMBO: UINT = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST;
pub const COMBO_EX: UINT = WS_EX_CLIENTEDGE;
/// A button of the standard height.
pub const BUTTON: UINT = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON;

// ------------------------------------------------------------------ painting

/// Colours of one look (`settings::Settings::theme`).
///
/// Every window of the program paints with one of these.  The system brushes are
/// not enough: they follow the system's own colours, which stay light in the night
/// look, and a classic window cannot be asked for a dark palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Background of every window.
    pub window: DWORD,
    /// Text of a window and of the list.
    pub text: DWORD,
    /// Secondary text: the status line, the word in front of a filter.
    pub muted: DWORD,
    /// Background of the list itself.
    pub list: DWORD,
    /// Background of a text field, a combo box or a read-only field.
    pub field: DWORD,
    /// Face of a button.
    pub face: DWORD,
    /// Background and text of a selected row while the list has the focus.
    pub selection: DWORD,
    pub selection_text: DWORD,
    /// The same for a list that does not have the focus right now.
    pub selection_idle: DWORD,
    pub selection_idle_text: DWORD,
    /// Thin rule between the sections.
    pub rule: DWORD,
    /// True when this is the night look.
    pub dark: bool,
}

/// `COLORREF` out of its three channels (the Windows `RGB` macro).
pub const fn rgb(red: u8, green: u8, blue: u8) -> DWORD {
    (red as DWORD) | ((green as DWORD) << 8) | ((blue as DWORD) << 16)
}

impl Palette {
    /// The look Windows draws by itself - what the program always had.
    pub fn light() -> Palette {
        unsafe {
            Palette {
                window: GetSysColor(COLOR_WINDOW),
                text: GetSysColor(COLOR_WINDOWTEXT),
                muted: GetSysColor(COLOR_GRAYTEXT),
                list: GetSysColor(COLOR_WINDOW),
                field: GetSysColor(COLOR_WINDOW),
                face: GetSysColor(COLOR_BTNFACE),
                selection: GetSysColor(COLOR_HIGHLIGHT),
                selection_text: GetSysColor(COLOR_HIGHLIGHTTEXT),
                // An unfocused list shows the selection in the button face colour;
                // that is what the eye expects when the window loses the focus.
                selection_idle: GetSysColor(COLOR_BTNFACE),
                selection_idle_text: GetSysColor(COLOR_BTNTEXT),
                rule: GetSysColor(COLOR_3DSHADOW),
                dark: false,
            }
        }
    }

    /// The night look: dark surfaces, light text.
    ///
    /// The numbers are the ones Windows itself uses for its dark shell, so the
    /// window does not introduce a second kind of dark next to it.
    pub fn dark() -> Palette {
        Palette {
            window: rgb(32, 32, 32),
            text: rgb(243, 243, 243),
            muted: rgb(160, 160, 160),
            list: rgb(24, 24, 24),
            // A field is a little lighter than the window around it: that is what
            // says "here you can type" without drawing a frame of our own.
            field: rgb(45, 45, 45),
            face: rgb(50, 50, 50),
            selection: rgb(0, 95, 184),
            selection_text: rgb(255, 255, 255),
            selection_idle: rgb(58, 58, 58),
            selection_idle_text: rgb(232, 232, 232),
            rule: rgb(76, 76, 76),
            dark: true,
        }
    }

    /// The palette of a stored setting (`"light"` or `"dark"`).
    pub fn from_setting(theme: &str) -> Palette {
        if theme == "dark" {
            Palette::dark()
        } else {
            Palette::light()
        }
    }

    /// Background and text of one row: selected or not, focused or not.
    pub fn row_colours(&self, selected: bool, focused: bool) -> (DWORD, DWORD) {
        match (selected, focused) {
            (true, true) => (self.selection, self.selection_text),
            (true, false) => (self.selection_idle, self.selection_idle_text),
            (false, _) => (self.list, self.text),
        }
    }
}

/// Brushes for solid colours, created once and kept for the life of the process.
///
/// A label asks for its background on every repaint, and a brush created per paint
/// would leak one GDI object per repaint.
pub fn brush(colour: DWORD) -> HBRUSH {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static BRUSHES: OnceLock<Mutex<HashMap<DWORD, usize>>> = OnceLock::new();
    let store = BRUSHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut store = match store.lock() {
        Ok(store) => store,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(handle) = store.get(&colour) {
        return *handle as HBRUSH;
    }
    let created = unsafe { CreateSolidBrush(colour) };
    store.insert(colour, created as usize);
    created
}

/// Apply the chosen look to one of the secondary windows.
///
/// Their controls are painted by Windows, so the night look reaches them through the
/// theme engine; where the system does not support that, they stay as they are.  The
/// surface of the window itself is painted by its own `WM_ERASEBKGND` handler.
pub fn apply_look_to_window(hwnd: HWND, palette: &Palette, controls: &[HWND]) {
    if !palette.dark {
        return;
    }
    dark_title_bar(hwnd, true);
    for control in controls {
        theme_control(*control, true);
    }
}

/// Paint the background of a window (`WM_ERASEBKGND`).
///
/// Windows would use the brush of the window class, and that brush belongs to the
/// system's own look: the night look has to paint the surface itself.
pub fn erase_background(hwnd: HWND, dc: HDC, palette: &Palette) -> LRESULT {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
        FillRect(dc, &rect, window_brush(palette));
    }
    1
}

/// The brush a window paints its background with.
pub fn window_brush(palette: &Palette) -> HBRUSH {
    brush(palette.window)
}

/// Answer `WM_CTLCOLORSTATIC` for a label: no background of its own.
///
/// Returning the window brush is what removes the grey rectangles; setting the
/// mode to transparent keeps the text from being drawn on a filled box that the
/// control then shifts by a pixel or two.
pub fn paint_label(dc: HDC, palette: &Palette, muted: bool) -> HBRUSH {
    let colour = if muted { palette.muted } else { palette.text };
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, colour);
    }
    window_brush(palette)
}

/// Answer the colour question of a control that is not a plain label.
///
/// A read-only field asks with `WM_CTLCOLORSTATIC` as well, and it is not painted on
/// the window but on the surface of a field: in the night look that is a different
/// colour, and a white box in a dark window would be the first thing the eye sees.
pub fn paint_field(dc: HDC, palette: &Palette) -> HBRUSH {
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, palette.text);
    }
    brush(palette.field)
}

/// Background of a control that answers `WM_CTLCOLORBTN`.
pub fn paint_button(dc: HDC, palette: &Palette) -> HBRUSH {
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, palette.text);
    }
    brush(palette.face)
}

/// Answer the colour question of a field, a dropped down list or a button.
///
/// One helper for the three messages: the difference is only which surface the
/// control paints on.
pub fn paint_surface(message: UINT, dc: HDC, palette: &Palette) -> HBRUSH {
    if message == WM_CTLCOLORBTN {
        paint_button(dc, palette)
    } else {
        paint_field(dc, palette)
    }
}

/// Is this one of the controls that paint on the surface of a field?
pub fn is_field(control: HWND) -> bool {
    let name = class_name(control);
    name.eq_ignore_ascii_case("Edit") || name.eq_ignore_ascii_case("ComboBox")
}

/// Answer one of the `WM_CTLCOLOR…` messages for a control.
///
/// `None` means "not a control this window paints": the caller then falls back on
/// the default handling of Windows.
pub fn paint_control(dc: HDC, control: HWND, palette: &Palette, muted: bool) -> Option<HBRUSH> {
    if is_field(control) {
        return Some(paint_field(dc, palette));
    }
    if on_window_background(control) {
        return Some(paint_label(dc, palette, muted));
    }
    None
}

/// Is this control a text field?  (Ctrl+A has to be handled by hand for those.)
pub fn is_edit(control: HWND) -> bool {
    class_name(control).eq_ignore_ascii_case("Edit")
}

/// Class name of a window (empty when it cannot be read).
fn class_name(control: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(control, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

/// Does this control ask for its background through `WM_CTLCOLORSTATIC`?
///
/// Static labels do - and so do check boxes and radio buttons.  A check box that
/// is answered with the button face colour paints a grey band across the whole
/// window: that is the "fill under the text" this look exists to remove, and the
/// band never lines up with the row next to it.
pub fn on_window_background(control: HWND) -> bool {
    let name = class_name(control);
    name.eq_ignore_ascii_case("Static") || name.eq_ignore_ascii_case("Button")
}

// ------------------------------------------------------------------- creation

/// Create one child control and give it the window font.
pub fn child(
    parent: HWND,
    instance: HINSTANCE,
    class: &str,
    text: &str,
    style: UINT,
    ex_style: UINT,
    id: usize,
    font: HFONT,
) -> HWND {
    let class_name = wide(class);
    let title = wide(text);
    let control = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            title.as_ptr(),
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
    };
    if !font.is_null() {
        unsafe {
            SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
        }
    }
    control
}

/// A label on the line of the control that follows it.
///
/// Whether it counts as secondary text is decided when it is painted (see
/// `paint_label`), so the id has to be one the window knows.
pub fn label(parent: HWND, instance: HINSTANCE, font: HFONT, text: &str, id: usize) -> HWND {
    child(parent, instance, "STATIC", text, LABEL, 0, id, font)
}

/// A thin rule across the window.
///
/// Owner drawn: the etched frame a static control would draw uses the system
/// colours, which belong to the day look - the night look paints the line itself.
pub fn separator(parent: HWND, instance: HINSTANCE, id: usize) -> HWND {
    child(
        parent,
        instance,
        "STATIC",
        "",
        WS_CHILD | WS_VISIBLE | SS_OWNERDRAW,
        0,
        id,
        std::ptr::null_mut(),
    )
}

/// How tall a drop down list wants to be: the field plus room for the list.
pub fn combo_height(scale: impl Fn(i32) -> i32) -> i32 {
    scale(ROW) + scale(160)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is background stays darker than what is text - in both looks, that is
    /// what makes a look readable at all.
    fn brightness(colour: DWORD) -> i32 {
        ((colour & 0xFF) + ((colour >> 8) & 0xFF) + ((colour >> 16) & 0xFF)) as i32
    }

    #[test]
    fn a_night_look_is_dark_and_the_day_look_is_not() {
        let day = Palette::light();
        let night = Palette::dark();
        assert!(!day.dark);
        assert!(night.dark);

        for palette in [day, night] {
            let background = brightness(palette.window);
            let text = brightness(palette.text);
            assert_ne!(background, text, "{palette:?}");
            let (low, high) = if background < text {
                (background, text)
            } else {
                (text, background)
            };
            // Notes and rules sit between the background and the text: one step away
            // from the surface, one step away from being as loud as the text.
            for (name, colour) in [("note", palette.muted), ("rule", palette.rule)] {
                let value = brightness(colour);
                assert!(
                    low < value && value < high,
                    "the {name} is not between background and text: {palette:?}"
                );
            }
            assert_ne!(
                palette.selection, palette.selection_idle,
                "a selected row has to look different when the list loses the focus"
            );
            assert!(
                brightness(palette.selection_text) != brightness(palette.selection),
                "the text of a selected row has to be readable: {palette:?}"
            );
        }

        assert!(
            brightness(night.window) < brightness(day.window),
            "the night look is the darker one"
        );
        assert!(
            brightness(night.text) > brightness(day.text),
            "and its text is the lighter one"
        );
    }

    /// A row asks the palette for its colours, so a selected row of an unfocused
    /// list does not look like a focused one.
    #[test]
    fn a_row_takes_its_colours_from_the_look() {
        let palette = Palette::dark();
        assert_eq!(
            palette.row_colours(false, true),
            (palette.list, palette.text)
        );
        assert_eq!(
            palette.row_colours(false, false),
            (palette.list, palette.text)
        );
        assert_eq!(
            palette.row_colours(true, true),
            (palette.selection, palette.selection_text)
        );
        assert_eq!(
            palette.row_colours(true, false),
            (palette.selection_idle, palette.selection_idle_text)
        );
    }

    #[test]
    fn a_stored_look_has_a_palette() {
        assert_eq!(Palette::from_setting("dark"), Palette::dark());
        assert_eq!(Palette::from_setting("light"), Palette::light());
        assert_eq!(
            Palette::from_setting("nonsense"),
            Palette::light(),
            "an unknown look falls back on the day look"
        );
        assert_eq!(rgb(0, 95, 184), 0x00B8_5F00);
    }
}
