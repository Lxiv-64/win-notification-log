//! The question that comes before an irreversible action.
//!
//! `MessageBoxW` cannot do this job well.  Its buttons are always «Да» and «Нет»
//! (or «ОК» and «Отмена»), the labels cannot be changed, and the only way to move
//! the focus away from the first button is `MB_DEFBUTTON2` - which puts it on
//! *cancel*.  That produced exactly the wrong question: a deletion asked with a
//! "no" that already held the focus, so pressing Enter kept the records instead
//! of deleting them, and «Да» alone does not say what it will do.
//!
//! This window asks in the words of the action ("Удалить 3 записи?") and puts the
//! focus on the button that carries it out: Enter confirms, Escape cancels, and
//! a stray Enter never leaves the user wondering what was pressed.

use std::cell::RefCell;
use std::rc::Rc;

use crate::style;
use crate::win32::*;

/// Logical pixel to monitor pixel.
fn px(value: i32, dpi: u32) -> i32 {
    scale(value, dpi)
}

const WINDOW_CLASS: &str = "WinNotificationLogConfirm";
const ID_QUESTION: usize = 901;
const ID_CONFIRM: usize = 902;
const ID_CANCEL: usize = 903;

/// Width of the question, in logical pixels.
const WIDTH: i32 = 470;
/// Width of a button of this dialog.
const BUTTON_WIDTH: i32 = 132;
/// How tall the question is when it cannot be measured (never happens in
/// practice: `DrawTextW` gives the exact height).
const FALLBACK_LINE_HEIGHT: i32 = 18;

/// State of the question window.
struct Question {
    hwnd: HWND,
    owner: HWND,
    dpi: u32,
    font: HFONT,
    /// Colours of the look in use (see `settings::Settings::theme`).
    palette: style::Palette,
    question: HWND,
    confirm_button: HWND,
    cancel_button: HWND,
    /// The answer, filled in when the user pressed one of the buttons.
    answer: Rc<RefCell<bool>>,
}

impl Drop for Question {
    fn drop(&mut self) {
        if !self.font.is_null() {
            unsafe {
                DeleteObject(self.font as HGDIOBJ);
            }
        }
    }
}

/// Ask a question with two buttons; true when the action was confirmed.
///
/// The question has to name what goes away and how much of it: "вы уверены?" is
/// not a question anybody can answer, and the next step cannot be undone.
/// `confirm` is the verb of the action («Удалить»), the other button is always
/// «Отмена».
pub fn ask(owner: HWND, question: &str, confirm: &str, dpi: u32, palette: style::Palette) -> bool {
    let dpi = dpi.max(96);
    let raw = Box::into_raw(Box::new(Question {
        hwnd: std::ptr::null_mut(),
        owner,
        dpi,
        font: system_font_scaled(dpi),
        palette,
        question: std::ptr::null_mut(),
        confirm_button: std::ptr::null_mut(),
        cancel_button: std::ptr::null_mut(),
        answer: Rc::new(RefCell::new(false)),
    }));
    register_class();
    // The answer outlives the window: the state behind `raw` is dropped by
    // `WM_NCDESTROY`, so the shared cell is kept here.
    let answer = unsafe { (*raw).answer.clone() };
    unsafe {
        let class = wide(WINDOW_CLASS);
        let title = wide(crate::appname::APPLICATION_TITLE);
        let font = (*raw).font;
        let inner = px(WIDTH, dpi);
        // The height follows the question: a measured box, not a guess.
        let text_height = measure_text_height(question, font, inner);
        let client_height = px(style::PADDING * 2, dpi)
            + text_height
            + px(style::SECTION_GAP, dpi)
            + px(style::ROW, dpi);
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: inner,
            bottom: client_height,
        };
        AdjustWindowRectEx(
            &mut frame,
            WS_CAPTION | WS_SYSMENU,
            0,
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        );
        let width = frame.width();
        let height = frame.height();
        // In the middle, a third of the way down: a question is not a window the
        // user has to look for.  Without an owner there is nothing to centre on.
        let (x, y) = if owner.is_null() {
            (CW_USEDEFAULT, CW_USEDEFAULT)
        } else {
            let mut owner_rect = RECT::default();
            GetWindowRect(owner, &mut owner_rect);
            (
                owner_rect.left + (owner_rect.width() - width) / 2,
                owner_rect.top + (owner_rect.height() - height) / 3,
            )
        };
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            owner,
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
        state.create_children(question, confirm);
        state.layout();
        style::apply_look_to_window(
            hwnd,
            &state.palette,
            &[state.question, state.confirm_button, state.cancel_button],
        );
        if !owner.is_null() {
            EnableWindow(owner, 0);
        }
        // The focus is on the button that carries out the action, not on "no".
        SetFocus(state.confirm_button);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        // Modal: the caller waits until the question is answered.
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let status = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
            if status <= 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &mut message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if !owner.is_null() {
            EnableWindow(owner, 1);
            SetForegroundWindow(owner);
        }
        let confirmed = *answer.borrow();
        confirmed
    }
}

/// Height the question needs when it is wrapped into `width` pixels.
///
/// A fixed number of lines cannot work: the question carries the titles of the
/// records, so it can be one line or six, and the text size of the interface
/// changes the height as well.
fn measure_text_height(question: &str, font: HFONT, width: i32) -> i32 {
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return 3 * FALLBACK_LINE_HEIGHT;
        }
        let previous = SelectObject(dc, font as HGDIOBJ);
        let mut text = wide(question);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: 0,
        };
        let flags = DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX;
        DrawTextW(dc, text.as_mut_ptr(), -1, &mut rect, flags);
        SelectObject(dc, previous);
        ReleaseDC(std::ptr::null_mut(), dc);
        // A pixel of slack: a wrapped line that rounds down clips its tail.
        rect.height().max(1) + 2
    }
}

impl Question {
    /// Build the question and the two buttons.
    fn create_children(&mut self, question: &str, confirm: &str) {
        let instance = module_handle();
        let hwnd = self.hwnd;
        self.question = style::child(
            hwnd,
            instance,
            "STATIC",
            question,
            WS_CHILD | WS_VISIBLE | style::HINT,
            0,
            ID_QUESTION,
            self.font,
        );
        self.confirm_button = style::child(
            hwnd,
            instance,
            "BUTTON",
            confirm,
            style::BUTTON | BS_DEFPUSHBUTTON,
            0,
            ID_CONFIRM,
            self.font,
        );
        self.cancel_button = style::child(
            hwnd,
            instance,
            "BUTTON",
            "Отмена",
            style::BUTTON,
            0,
            ID_CANCEL,
            self.font,
        );
    }

    /// Place the question and the buttons; the buttons sit at the right edge.
    fn layout(&self) {
        let dpi = self.dpi;
        let padding = px(style::PADDING, dpi);
        let gap = px(style::GAP, dpi);
        let button_width = px(BUTTON_WIDTH, dpi);
        let button_height = px(style::ROW, dpi);
        let mut client = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let width = client.width();
        let height = client.height();
        let buttons_top = height - padding - button_height;
        let text_height = (buttons_top - px(style::SECTION_GAP, dpi) - padding).max(1);
        unsafe {
            MoveWindow(
                self.question,
                padding,
                padding,
                (width - 2 * padding).max(1),
                text_height,
                1,
            );
            MoveWindow(
                self.cancel_button,
                width - padding - button_width,
                buttons_top,
                button_width,
                button_height,
                1,
            );
            MoveWindow(
                self.confirm_button,
                width - padding - 2 * button_width - gap,
                buttons_top,
                button_width,
                button_height,
                1,
            );
        }
    }

    /// Remember the answer and close the window.
    fn answer(&self, confirmed: bool) {
        *self.answer.borrow_mut() = confirmed;
        unsafe {
            DestroyWindow(self.hwnd);
        }
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

/// State of a window from its `GWLP_USERDATA`.
unsafe fn state_of(hwnd: HWND) -> Option<&'static mut Question> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Question;
    raw.as_mut()
}

/// Window procedure of the question window.
unsafe extern "system" fn window_proc(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        // The question is plain text on the window background.
        WM_ERASEBKGND => match state_of(hwnd) {
            Some(state) => style::erase_background(hwnd, w as HDC, &state.palette),
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CTLCOLORSTATIC => {
            let control = l as HWND;
            match state_of(hwnd).map(|state| state.palette) {
                Some(palette) => match style::paint_control(w as HDC, control, &palette, false) {
                    Some(background) => background as LRESULT,
                    None => DefWindowProcW(hwnd, message, w, l),
                },
                None => DefWindowProcW(hwnd, message, w, l),
            }
        }
        WM_COMMAND => {
            let id = (w & 0xFFFF) as usize;
            if let Some(state) = state_of(hwnd) {
                match id {
                    ID_CONFIRM => state.answer(true),
                    ID_CANCEL => state.answer(false),
                    _ => {}
                }
            }
            0
        }
        WM_KEYDOWN => {
            // The focus is on «Удалить», so Enter confirms; Escape is the way out
            // that cannot destroy anything.
            if w as UINT == VK_ESCAPE {
                if let Some(state) = state_of(hwnd) {
                    state.answer(false);
                }
                0
            } else {
                DefWindowProcW(hwnd, message, w, l)
            }
        }
        WM_CLOSE => {
            if let Some(state) = state_of(hwnd) {
                state.answer(false);
            }
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Question;
            if !raw.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_window_owns_its_buttons() {
        // This dialog exists because the labels of a message box cannot be chosen
        // and because the focus has to sit on the action; these are the promises
        // that have to hold whatever else changes.
        assert_ne!(ID_CONFIRM, ID_CANCEL);
        assert_ne!(
            ID_CONFIRM, IDOK,
            "the ids stay out of the range Windows uses for its own buttons"
        );
        assert_ne!(ID_CANCEL, IDCANCEL);
    }
}
