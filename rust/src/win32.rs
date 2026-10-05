//! Minimal hand written Win32 bindings used by the graphical build.
//!
//! Only the surface the application really uses is declared here, which keeps the
//! dependency set at zero external crates: the compiler links straight against the
//! system libraries (user32, comctl32, shell32, gdi32, kernel32) that ship with
//! Windows, exactly like `sqlite_ffi` takes SQLite from `winsqlite3.dll`.

#![allow(
    non_snake_case,
    non_camel_case_types,
    dead_code,
    clippy::upper_case_acronyms
)]

use std::ffi::c_void;

pub type HWND = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HMENU = *mut c_void;
pub type HICON = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HBRUSH = *mut c_void;
pub type HFONT = *mut c_void;
pub type HGDIOBJ = *mut c_void;
pub type HBITMAP = *mut c_void;
pub type HANDLE = *mut c_void;
pub type HMODULE = *mut c_void;
pub type HIMAGELIST = *mut c_void;
/// Handle of a resource inside an executable (see `FindResourceW`).
pub type HRSRC = *mut c_void;
pub type HDC = *mut c_void;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type BOOL = i32;
pub type LPCWSTR = *const u16;
pub type LPWSTR = *mut u16;
pub type ATOM = u16;
pub type UINT = u32;
pub type DWORD = u32;

// ------------------------------------------------------------------- structures
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RECT {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Width and height of a measured box (`GetTextExtentPoint32W`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

/// How tall the owner wants one row of a control that draws itself
/// (`WM_MEASUREITEM`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MEASUREITEMSTRUCT {
    pub ctl_type: u32,
    pub ctl_id: u32,
    pub item_id: u32,
    pub item_width: u32,
    pub item_height: u32,
    pub item_data: usize,
}

/// One row that a control asks its owner to draw (`WM_DRAWITEM`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DRAWITEMSTRUCT {
    pub ctl_type: u32,
    pub ctl_id: u32,
    pub item_id: u32,
    pub item_action: u32,
    pub item_state: u32,
    pub hwnd_item: HWND,
    pub hdc: HDC,
    pub rc_item: RECT,
    pub item_data: usize,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: UINT,
    pub w_param: WPARAM,
    pub l_param: LPARAM,
    pub time: DWORD,
    pub pt: POINT,
    pub l_private: DWORD,
}

pub type WNDPROC = Option<unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT>;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cb_size: UINT,
    pub style: UINT,
    pub lpfn_wnd_proc: WNDPROC,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: HICON,
    pub h_cursor: HCURSOR,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: LPCWSTR,
    pub lpsz_class_name: LPCWSTR,
    pub h_icon_sm: HICON,
}

impl Default for WNDCLASSEXW {
    fn default() -> Self {
        WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as UINT,
            style: 0,
            lpfn_wnd_proc: None,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: std::ptr::null_mut(),
            h_icon: std::ptr::null_mut(),
            h_cursor: std::ptr::null_mut(),
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: std::ptr::null(),
            h_icon_sm: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
pub struct INITCOMMONCONTROLSEX {
    pub dw_size: DWORD,
    pub dw_icc: DWORD,
}

/// One column of a report mode list view.
#[repr(C)]
pub struct LVCOLUMNW {
    pub mask: UINT,
    pub fmt: i32,
    pub cx: i32,
    pub psz_text: LPWSTR,
    pub cch_text_max: i32,
    pub i_sub_item: i32,
    pub i_image: i32,
    pub i_order: i32,
    pub cx_min: i32,
    pub cx_default: i32,
    pub cx_ideal: i32,
}

/// All fields are integers or pointers, so the zeroed bit pattern is valid.
impl Default for LVCOLUMNW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// One row of a report mode list view.
#[repr(C)]
pub struct LVITEMW {
    pub mask: UINT,
    pub i_item: i32,
    pub i_sub_item: i32,
    pub state: UINT,
    pub state_mask: UINT,
    pub psz_text: LPWSTR,
    pub cch_text_max: i32,
    pub i_image: i32,
    pub l_param: LPARAM,
    pub i_indent: i32,
    pub i_group_id: i32,
    pub c_columns: UINT,
    pub pu_columns: *mut UINT,
    pub pi_col_fmt: *mut i32,
    pub i_group: i32,
}

/// All fields are integers or pointers, so the zeroed bit pattern is valid.
impl Default for LVITEMW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
pub struct NMHDR {
    pub hwnd_from: HWND,
    pub id_from: usize,
    pub code: UINT,
}

/// Notification codes shared by the common controls.
///
/// They count *down* from `NM_FIRST`, which is why they are large unsigned
/// values; the list view adds its own offset (`LVN_*`).
pub const NM_FIRST: UINT = 0;
pub const NM_CLICK: UINT = NM_FIRST.wrapping_sub(2);
pub const NM_DBLCLK: UINT = NM_FIRST.wrapping_sub(3);
pub const NM_RCLICK: UINT = NM_FIRST.wrapping_sub(5);
pub const NM_RDBLCLK: UINT = NM_FIRST.wrapping_sub(6);

#[repr(C)]
pub struct NMLISTVIEW {
    pub hdr: NMHDR,
    pub i_item: i32,
    pub i_sub_item: i32,
    pub u_new_state: UINT,
    pub u_old_state: UINT,
    pub u_changed: UINT,
    pub pt_action: POINT,
    pub l_param: LPARAM,
}

/// `WM_NOTIFY` payload of a key press inside a list view (`NMLVKEYDOWN`).
#[repr(C)]
pub struct NMLVKEYDOWN {
    pub hdr: NMHDR,
    pub v_key: u16,
    pub flags: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

/// Tray icon description (`NOTIFYICONDATAW`, Vista layout).
#[repr(C)]
pub struct NOTIFYICONDATAW {
    pub cb_size: DWORD,
    pub hwnd: HWND,
    pub u_id: UINT,
    pub u_flags: UINT,
    pub u_callback_message: UINT,
    pub h_icon: HICON,
    pub sz_tip: [u16; 128],
    pub dw_state: DWORD,
    pub dw_state_mask: DWORD,
    pub sz_info: [u16; 256],
    pub u_timeout_or_version: UINT,
    pub sz_info_title: [u16; 64],
    pub dw_info_flags: DWORD,
    pub guid_item: GUID,
    pub h_balloon_icon: HICON,
}

impl Default for NOTIFYICONDATAW {
    fn default() -> Self {
        NOTIFYICONDATAW {
            cb_size: std::mem::size_of::<NOTIFYICONDATAW>() as DWORD,
            hwnd: std::ptr::null_mut(),
            u_id: 0,
            u_flags: 0,
            u_callback_message: 0,
            h_icon: std::ptr::null_mut(),
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_or_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid_item: GUID::default(),
            h_balloon_icon: std::ptr::null_mut(),
        }
    }
}

impl NOTIFYICONDATAW {
    pub fn set_tip(&mut self, text: &str) {
        fill_wide(&mut self.sz_tip, text);
    }
}

// -------------------------------------------------------------------- messages
pub const WM_CREATE: UINT = 0x0001;
/// The first message a window receives: the last parameter of `CreateWindowExW`
/// travels inside `lParam` here (see `ui::window_proc`).
pub const WM_NCCREATE: UINT = 0x0081;
pub const WM_DESTROY: UINT = 0x0002;
pub const WM_SIZE: UINT = 0x0005;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_SETFONT: UINT = 0x0030;
pub const WM_COMMAND: UINT = 0x0111;
pub const WM_TIMER: UINT = 0x0113;
pub const WM_NOTIFY: UINT = 0x004E;
pub const WM_KEYDOWN: UINT = 0x0100;
pub const WM_NCDESTROY: UINT = 0x0082;
pub const WM_LBUTTONDBLCLK: UINT = 0x0203;
pub const WM_RBUTTONUP: UINT = 0x0205;
pub const WM_CONTEXTMENU: UINT = 0x007B;
pub const WM_APP: UINT = 0x8000;
/// Posted when the journal changed and the listing should be reloaded.
pub const WM_APP_REFRESH: UINT = WM_APP + 1;
/// Callback message of the tray icon.
pub const WM_APP_TRAY: UINT = WM_APP + 2;
/// Posted by a second start of the program to show the existing window.
pub const WM_APP_SHOW: UINT = WM_APP + 10;

pub const EM_SETSEL: UINT = 0x00B1;
pub const EM_SETLIMITTEXT: UINT = 0x00C5;
/// Edit control notification: the text changed.
pub const EN_CHANGE: UINT = 0x0300;
/// Button notification: pressed and released.
pub const BN_CLICKED: UINT = 0x0000;
/// Combo box notification: another entry was chosen.
pub const CBN_SELCHANGE: UINT = 0x0001;

// --------------------------------------------------------------- window styles
pub const WS_OVERLAPPEDWINDOW: UINT = 0x00CF_0000;
pub const WS_CHILD: UINT = 0x4000_0000;
pub const WS_VISIBLE: UINT = 0x1000_0000;
pub const WS_TABSTOP: UINT = 0x0001_0000;
pub const WS_BORDER: UINT = 0x0080_0000;
pub const WS_VSCROLL: UINT = 0x0020_0000;
pub const WS_CLIPCHILDREN: UINT = 0x0200_0000;
pub const WS_EX_CLIENTEDGE: UINT = 0x0000_0200;
pub const WS_EX_CONTROLPARENT: UINT = 0x0001_0000;
pub const WS_EX_DLGMODALFRAME: UINT = 0x0000_0001;
pub const WS_EX_TOPMOST: UINT = 0x0000_0008;
pub const WS_OVERLAPPED: UINT = 0x0000_0000;
pub const WS_CAPTION: UINT = 0x00C0_0000;
pub const WS_SYSMENU: UINT = 0x0008_0000;
pub const CBS_DROPDOWNLIST: UINT = 0x0003;
pub const CB_ADDSTRING: UINT = 0x0143;
pub const CB_GETCURSEL: UINT = 0x0147;
pub const CB_SETCURSEL: UINT = 0x014E;
pub const CB_RESETCONTENT: UINT = 0x014B;
pub const CW_USEDEFAULT: i32 = -2147483648;

pub const ES_LEFT: UINT = 0x0000;
pub const ES_MULTILINE: UINT = 0x0004;
pub const ES_AUTOVSCROLL: UINT = 0x0040;
pub const ES_AUTOHSCROLL: UINT = 0x0080;
pub const ES_READONLY: UINT = 0x0800;
pub const ES_WANTRETURN: UINT = 0x1000;
pub const SS_LEFT: UINT = 0x0000;
pub const SS_RIGHT: UINT = 0x0002;
pub const SS_SUNKEN: UINT = 0x0000_1000;
/// Centre single line text vertically inside the control.
///
/// A static that is given the very same box as the field next to it puts its
/// text on exactly the same line as that field - no hand written offset that
/// breaks as soon as the text size changes.
pub const SS_CENTERIMAGE: UINT = 0x0000_0200;
/// Draw the text as it is: `&` is not a mnemonic.
///
/// Notification titles and bodies arrive from other programs and a literal `&`
/// in one of them would otherwise be swallowed - or underline the next letter.
pub const SS_NOPREFIX: UINT = 0x0080;
/// Replace the end of a too long line with an ellipsis instead of clipping it.
pub const SS_ENDELLIPSIS: UINT = 0x0000_4000;
/// A one pixel high etched frame: the thin rule that separates areas.
pub const SS_ETCHEDHORZ: UINT = 0x0000_0010;
pub const BS_PUSHBUTTON: UINT = 0x0000;
/// The button that Enter activates while nothing else has the focus.
pub const BS_DEFPUSHBUTTON: UINT = 0x0000_0001;

// ------------------------------------------------------------------ list view
pub const LVS_REPORT: UINT = 0x0001;
pub const LVS_SINGLESEL: UINT = 0x0004;
pub const LVS_SHOWSELALWAYS: UINT = 0x0008;
pub const LVS_EX_FULLROWSELECT: UINT = 0x0000_0020;
pub const LVS_EX_DOUBLEBUFFER: UINT = 0x0001_0000;

pub const LVM_FIRST: UINT = 0x1000;
/// Background of the area a report does not fill with rows; a list that is painted
/// dark has to say so, otherwise a white band shows under the last row.
pub const LVM_SETBKCOLOR: UINT = LVM_FIRST + 1;
pub const LVM_SETTEXTBKCOLOR: UINT = LVM_FIRST + 38;
pub const LVM_SETTEXTCOLOR: UINT = LVM_FIRST + 36;
/// The header control of a report (`LVM_GETHEADER`).
pub const LVM_GETHEADER: UINT = LVM_FIRST + 31;
pub const LVM_GETITEMCOUNT: UINT = LVM_FIRST + 4;
pub const LVM_DELETEALLITEMS: UINT = LVM_FIRST + 9;
pub const LVM_GETNEXTITEM: UINT = LVM_FIRST + 12;
pub const LVM_ENSUREVISIBLE: UINT = LVM_FIRST + 19;
pub const LVM_SETCOLUMNWIDTH: UINT = LVM_FIRST + 30;
pub const LVM_SETITEMSTATE: UINT = LVM_FIRST + 43;
pub const LVM_SETEXTENDEDLISTVIEWSTYLE: UINT = LVM_FIRST + 54;
pub const LVM_INSERTITEMW: UINT = LVM_FIRST + 77;
pub const LVM_SETITEMW: UINT = LVM_FIRST + 76;
pub const LVM_INSERTCOLUMNW: UINT = LVM_FIRST + 97;
/// Drop one column; deleting column 0 repeatedly empties the report.
pub const LVM_DELETECOLUMN: UINT = LVM_FIRST + 28;

pub const LVNI_SELECTED: LPARAM = 0x0002;
pub const LVIF_TEXT: UINT = 0x0001;
pub const LVCF_FMT: UINT = 0x0001;
pub const LVCF_WIDTH: UINT = 0x0002;
pub const LVCF_TEXT: UINT = 0x0004;
pub const LVCF_SUBITEM: UINT = 0x0008;
pub const LVCFMT_LEFT: i32 = 0x0000;
pub const LVCFMT_RIGHT: i32 = 0x0001;
pub const LVIS_SELECTED: UINT = 0x0002;
pub const LVIS_FOCUSED: UINT = 0x0001;

pub const LVN_FIRST: UINT = 0xFFFF_FF9C; // -100
pub const LVN_ITEMCHANGED: UINT = LVN_FIRST.wrapping_sub(1);
pub const LVN_COLUMNCLICK: UINT = LVN_FIRST.wrapping_sub(8);

pub const ICC_LISTVIEW_CLASSES: DWORD = 0x0000_0001;
pub const ICC_BAR_CLASSES: DWORD = 0x0000_0004;
/// Tooltips inside a list view (list view extended style).
pub const LVS_EX_LABELTIP: UINT = 0x0000_4000;

// ------------------------------------------------------------ menus and shell
pub const MF_STRING: UINT = 0x0000;
pub const MF_POPUP: UINT = 0x0010;
pub const MF_SEPARATOR: UINT = 0x0800;
pub const MF_CHECKED: UINT = 0x0008;
pub const MF_UNCHECKED: UINT = 0x0000;
pub const MF_GRAYED: UINT = 0x0001;
pub const MF_BYCOMMAND: UINT = 0x0000;
pub const TPM_LEFTALIGN: UINT = 0x0000;
pub const TPM_RIGHTBUTTON: UINT = 0x0002;
pub const TPM_NONOTIFY: UINT = 0x0080;
pub const TPM_RETURNCMD: UINT = 0x0100;

pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOW: i32 = 5;
pub const SW_HIDE: i32 = 0;
pub const SW_RESTORE: i32 = 9;

pub const MB_OK: UINT = 0x0000;
pub const MB_YESNO: UINT = 0x0004;
pub const MB_DEFBUTTON2: UINT = 0x0100;
pub const MB_ICONERROR: UINT = 0x0010;
pub const MB_ICONQUESTION: UINT = 0x0020;
pub const MB_ICONWARNING: UINT = 0x0030;
pub const MB_ICONINFORMATION: UINT = 0x0040;
pub const IDYES: i32 = 6;
/// Standard button identifiers: what a dialog answers with.
pub const IDOK: usize = 1;
pub const IDCANCEL: usize = 2;

pub const GWLP_USERDATA: i32 = -21;
pub const COLOR_WINDOW_BRUSH: HBRUSH = 6usize as HBRUSH;
pub const IDI_APPLICATION: LPCWSTR = 32512usize as LPCWSTR;
pub const IDC_ARROW: LPCWSTR = 32512usize as LPCWSTR;
pub const DEFAULT_GUI_FONT: i32 = 17;

pub const NIM_ADD: DWORD = 0x0000;
pub const NIM_MODIFY: DWORD = 0x0001;
pub const NIM_DELETE: DWORD = 0x0002;
pub const NIF_MESSAGE: UINT = 0x0001;
pub const NIF_ICON: UINT = 0x0002;
pub const NIF_TIP: UINT = 0x0004;

pub const SM_CXSCREEN: i32 = 0;
pub const SM_CYSCREEN: i32 = 1;
/// Width of a vertical scroll bar; a list that hides one takes this much room.
pub const SM_CXVSCROLL: i32 = 2;
pub const PM_REMOVE: UINT = 0x0001;

// ------------------------------------------------------------------ interface
#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(class: *const WNDCLASSEXW) -> ATOM;
    pub fn CreateWindowExW(
        ex_style: UINT,
        class_name: LPCWSTR,
        window_name: LPCWSTR,
        style: UINT,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    pub fn DefWindowProcW(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT;
    pub fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: UINT, max: UINT) -> BOOL;
    pub fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: UINT, max: UINT, remove: UINT) -> BOOL;
    pub fn TranslateMessage(msg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    pub fn IsDialogMessageW(hwnd: HWND, msg: *mut MSG) -> BOOL;
    pub fn PostQuitMessage(code: i32);
    pub fn PostMessageW(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> BOOL;
    pub fn SendMessageW(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT;
    pub fn ShowWindow(hwnd: HWND, command: i32) -> BOOL;
    pub fn UpdateWindow(hwnd: HWND) -> BOOL;
    pub fn DestroyWindow(hwnd: HWND) -> BOOL;
    pub fn SetWindowTextW(hwnd: HWND, text: LPCWSTR) -> BOOL;
    pub fn GetWindowTextW(hwnd: HWND, buffer: LPWSTR, count: i32) -> i32;
    pub fn GetWindowTextLengthW(hwnd: HWND) -> i32;
    pub fn MessageBoxW(hwnd: HWND, text: LPCWSTR, caption: LPCWSTR, kind: UINT) -> i32;
    pub fn SetTimer(hwnd: HWND, id: usize, interval: UINT, callback: *mut c_void) -> usize;
    pub fn KillTimer(hwnd: HWND, id: usize) -> BOOL;
    pub fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn MoveWindow(hwnd: HWND, x: i32, y: i32, width: i32, height: i32, repaint: BOOL) -> BOOL;
    pub fn EnableWindow(hwnd: HWND, enable: BOOL) -> BOOL;
    pub fn SetFocus(hwnd: HWND) -> HWND;
    pub fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    pub fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    pub fn SetMenu(hwnd: HWND, menu: HMENU) -> BOOL;
    pub fn CreatePopupMenu() -> HMENU;
    pub fn CreateMenu() -> HMENU;
    pub fn AppendMenuW(menu: HMENU, flags: UINT, id: usize, item: LPCWSTR) -> BOOL;
    pub fn DestroyMenu(menu: HMENU) -> BOOL;
    pub fn TrackPopupMenu(
        menu: HMENU,
        flags: UINT,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: HWND,
        rect: *const RECT,
    ) -> BOOL;
    pub fn GetCursorPos(point: *mut POINT) -> BOOL;
    pub fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    pub fn LoadIconW(instance: HINSTANCE, name: LPCWSTR) -> HICON;
    pub fn LoadCursorW(instance: HINSTANCE, name: LPCWSTR) -> HCURSOR;
}

#[link(name = "comctl32")]
extern "system" {
    pub fn InitCommonControlsEx(classes: *const INITCOMMONCONTROLSEX) -> BOOL;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn GetStockObject(index: i32) -> HGDIOBJ;
}

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(message: DWORD, data: *mut NOTIFYICONDATAW) -> BOOL;
    pub fn ShellExecuteW(
        hwnd: HWND,
        operation: LPCWSTR,
        file: LPCWSTR,
        parameters: LPCWSTR,
        directory: LPCWSTR,
        show: i32,
    ) -> HINSTANCE;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: LPCWSTR) -> HINSTANCE;
    pub fn Sleep(milliseconds: DWORD);
    /// An embedded resource by identifier and type (`MAKEINTRESOURCE` values).
    pub fn FindResourceW(module: HINSTANCE, name: LPCWSTR, kind: LPCWSTR) -> HRSRC;
}

/// Handle of the current executable (used by the window class and controls).
pub fn module_handle() -> HINSTANCE {
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

/// A resource identifier where the API wants a pointer (`MAKEINTRESOURCE`).
///
/// Windows distinguishes numbered resources from named ones by the value itself,
/// so the number is passed as a pointer - exactly like `IDI_APPLICATION` below.
pub fn int_resource(id: u32) -> LPCWSTR {
    id as usize as LPCWSTR
}

// --------------------------------------------------------------------- helpers
/// NUL terminated UTF-16 buffer for the `*W` APIs.
///
/// Every window text, menu label, control caption and list cell travels through
/// here, so this is also the place where the interface language is applied: the
/// string is translated first (see `crate::lang`) and then encoded.  A string
/// that is not interface text - a class name, a path, journal data - is not in
/// the table and comes back unchanged.
pub fn wide(text: &str) -> Vec<u16> {
    crate::lang::tr(text)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

/// Copy a string into a fixed size UTF-16 buffer (keeps the NUL terminator).
///
/// Translated as well: this is the buffer of a tooltip or a file dialog.
pub fn fill_wide(buffer: &mut [u16], text: &str) {
    let text = crate::lang::tr(text);
    let limit = buffer.len().saturating_sub(1);
    buffer.iter_mut().for_each(|slot| *slot = 0);
    for (slot, unit) in buffer.iter_mut().zip(text.encode_utf16()).take(limit) {
        *slot = unit;
    }
}

/// Read a NUL terminated UTF-16 buffer into a `String`.
pub fn read_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Text of an edit control or a window caption.
pub fn window_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = GetWindowTextW(hwnd, buffer.as_mut_ptr(), length + 1);
        if written <= 0 {
            return String::new();
        }
        read_wide(&buffer[..written as usize])
    }
}

/// Set the text of a window or control.
pub fn set_window_text(hwnd: HWND, text: &str) {
    let buffer = wide(text);
    unsafe { SetWindowTextW(hwnd, buffer.as_ptr()) };
}

/// Re-interpret the `lParam` of `WM_NOTIFY` as a notification header.
///
/// # Safety
/// The pointer has to be a valid `NMHDR` chain supplied by the common controls.
pub unsafe fn notify_header(l_param: LPARAM) -> &'static NMHDR {
    &*(l_param as *const NMHDR)
}

// ====================================================== presentational additions
// Everything below extends the original surface with what a friendlier window
// needs: the system user interface font, per-monitor DPI, a status bar, image
// lists, application icons, file dialogs and picture previews.

/// One font description (`LOGFONTW`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LOGFONTW {
    pub lf_height: i32,
    pub lf_width: i32,
    pub lf_escapement: i32,
    pub lf_orientation: i32,
    pub lf_weight: i32,
    pub lf_italic: u8,
    pub lf_underline: u8,
    pub lf_strike_out: u8,
    pub lf_char_set: u8,
    pub lf_out_precision: u8,
    pub lf_clip_precision: u8,
    pub lf_quality: u8,
    pub lf_pitch_and_family: u8,
    pub lf_face_name: [u16; 32],
}

/// All fields are integers or arrays, so the zeroed bit pattern is valid.
impl Default for LOGFONTW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// The metrics Windows uses for its own windows (`NONCLIENTMETRICSW`).
#[repr(C)]
pub struct NONCLIENTMETRICSW {
    pub cb_size: UINT,
    pub i_border_width: i32,
    pub i_scroll_width: i32,
    pub i_scroll_height: i32,
    pub i_caption_width: i32,
    pub i_caption_height: i32,
    pub lf_caption_font: LOGFONTW,
    pub i_sm_caption_width: i32,
    pub i_sm_caption_height: i32,
    pub lf_sm_caption_font: LOGFONTW,
    pub i_menu_width: i32,
    pub i_menu_height: i32,
    pub lf_menu_font: LOGFONTW,
    pub lf_status_font: LOGFONTW,
    pub lf_message_font: LOGFONTW,
    pub i_padded_border_width: i32,
}

/// All fields are integers or arrays, so the zeroed bit pattern is valid.
impl Default for NONCLIENTMETRICSW {
    fn default() -> Self {
        let mut metrics: NONCLIENTMETRICSW = unsafe { std::mem::zeroed() };
        metrics.cb_size = std::mem::size_of::<NONCLIENTMETRICSW>() as UINT;
        metrics
    }
}

/// Activation context description (`ACTCTXW`), used to load common controls v6.
#[repr(C)]
pub struct ACTCTXW {
    pub cb_size: u32,
    pub dw_flags: u32,
    pub lp_source: LPCWSTR,
    pub w_processor_architecture: u16,
    pub w_lang_id: u16,
    pub lp_assembly_directory: LPCWSTR,
    pub lp_resource_name: LPCWSTR,
    pub lp_application_name: LPCWSTR,
    pub h_module: HMODULE,
    pub lp_assembly_identity: LPCWSTR,
}

/// All fields are integers or pointers, so the zeroed bit pattern is valid.
impl Default for ACTCTXW {
    fn default() -> Self {
        let mut context: ACTCTXW = unsafe { std::mem::zeroed() };
        context.cb_size = std::mem::size_of::<ACTCTXW>() as u32;
        context
    }
}

/// One entry of a process snapshot (`PROCESSENTRY32W`).
#[repr(C)]
pub struct PROCESSENTRY32W {
    pub dw_size: u32,
    pub cnt_usage: u32,
    pub th32_process_id: u32,
    pub th32_default_heap_id: usize,
    pub th32_module_id: u32,
    pub cnt_threads: u32,
    pub th32_parent_process_id: u32,
    pub pc_pri_class_base: i32,
    pub dw_flags: u32,
    pub sz_exe_file: [u16; 260],
}

impl Default for PROCESSENTRY32W {
    fn default() -> Self {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        entry
    }
}

/// File information returned by the shell (`SHFILEINFOW`).
#[repr(C)]
pub struct SHFILEINFOW {
    pub h_icon: HICON,
    pub i_icon: i32,
    pub dw_attributes: u32,
    pub sz_display_name: [u16; 260],
    pub sz_type_name: [u16; 80],
}

impl Default for SHFILEINFOW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// "Save as" dialog description (`OPENFILENAMEW`).
#[repr(C)]
pub struct OPENFILENAMEW {
    pub l_struct_size: u32,
    pub hwnd_owner: HWND,
    pub h_instance: HINSTANCE,
    pub lpstr_filter: LPCWSTR,
    pub lpstr_custom_filter: LPWSTR,
    pub n_max_cust_filter: u32,
    pub n_filter_index: u32,
    pub lpstr_file: LPWSTR,
    pub n_max_file: u32,
    pub lpstr_file_title: LPWSTR,
    pub n_max_file_title: u32,
    pub lpstr_initial_dir: LPCWSTR,
    pub lpstr_title: LPCWSTR,
    pub flags: u32,
    pub n_file_offset: u16,
    pub n_file_extension: u16,
    pub lpstr_def_ext: LPCWSTR,
    pub l_cust_data: LPARAM,
    pub lpfn_hook: *mut c_void,
    pub lp_template_name: LPCWSTR,
    pub pv_reserved: *mut c_void,
    pub dw_reserved: u32,
    pub flags_ex: u32,
}

impl Default for OPENFILENAMEW {
    fn default() -> Self {
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.l_struct_size = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog
    }
}

/// Folder picker description (`BROWSEINFOW`).
#[repr(C)]
pub struct BROWSEINFOW {
    pub hwnd_owner: HWND,
    pub pidl_root: *mut c_void,
    pub psz_display_name: LPWSTR,
    pub lpsz_title: LPCWSTR,
    pub ul_flags: u32,
    pub lpfn: *mut c_void,
    pub l_param: LPARAM,
    pub i_image: i32,
}

impl Default for BROWSEINFOW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// Minimum tracking size of a window (`MINMAXINFO`).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MINMAXINFO {
    pub reserved: POINT,
    pub max_size: POINT,
    pub max_position: POINT,
    pub min_track_size: POINT,
    pub max_track_size: POINT,
}

/// Startup description of GDI+ (`GdiplusStartupInput`).
#[repr(C)]
pub struct GDIPLUS_STARTUP_INPUT {
    pub version: u32,
    pub debug_callback: *mut c_void,
    pub suppress_background_thread: i32,
    pub suppress_external_codecs: i32,
}

impl Default for GDIPLUS_STARTUP_INPUT {
    fn default() -> Self {
        GDIPLUS_STARTUP_INPUT {
            version: 1,
            debug_callback: std::ptr::null_mut(),
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        }
    }
}

// ------------------------------------------------------------ extra constants
/// `SystemParametersInfoW`: read the metrics of the system's own windows.
pub const SPI_GETNONCLIENTMETRICS: UINT = 0x0029;
/// Image list used by the small icons of a list view.
pub const LVSIL_SMALL: i32 = 1;
pub const ILC_COLOR32: UINT = 0x0000_0020;
pub const LVM_SETIMAGELIST: UINT = LVM_FIRST + 3;
pub const LVIF_IMAGE: UINT = 0x0002;
/// The user activated (double clicked) an item.
pub const LVN_ITEMACTIVATE: UINT = LVN_FIRST.wrapping_sub(14);
/// A key was pressed while the list view had the focus (`NMLVKEYDOWN`).
pub const LVN_KEYDOWN: UINT = LVN_FIRST.wrapping_sub(55);
pub const VK_RETURN: UINT = 0x0D;
pub const VK_F: UINT = 0x46;
pub const VK_R: UINT = 0x52;
pub const VK_A: UINT = 0x41;
pub const VK_C: UINT = 0x43;
pub const VK_DELETE: UINT = 0x2E;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_TAB: UINT = 0x09;
pub const VK_ESCAPE: UINT = 0x1B;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_F5: UINT = 0x74;
pub const VK_S: UINT = 0x53;

/// Base of the messages a control class defines for itself (`WM_USER + n`).
pub const WM_USER: UINT = 0x0400;

/// Check boxes and group boxes of the settings window.
pub const BS_AUTOCHECKBOX: UINT = 0x0000_0003;
pub const BS_GROUPBOX: UINT = 0x0000_0007;
pub const WS_GROUP: UINT = 0x0002_0000;
pub const BM_SETCHECK: UINT = 0x00F1;
pub const BM_GETCHECK: UINT = 0x00F0;
pub const BST_CHECKED: WPARAM = 1;
pub const ES_NUMBER: UINT = 0x2000;

/// Static controls that display an icon or a bitmap.
pub const SS_ICON: UINT = 0x0000_0003;
pub const SS_BITMAP: UINT = 0x0000_000E;
pub const STM_SETIMAGE: UINT = 0x0172;
pub const IMAGE_BITMAP: WPARAM = 0;
pub const IMAGE_ICON: WPARAM = 1;
pub const WM_ERASEBKGND: UINT = 0x0014;
/// A control that draws itself asks its owner for the height of a row and for the
/// row itself with these two messages.
pub const WM_DRAWITEM: UINT = 0x002B;
pub const WM_MEASUREITEM: UINT = 0x002C;
pub const WM_CTLCOLORSTATIC: UINT = 0x0138;
/// Colours of the fields, lists and buttons of a window: the same question as
/// `WM_CTLCOLORSTATIC`, for the controls that are not labels.
pub const WM_CTLCOLOREDIT: UINT = 0x0133;
pub const WM_CTLCOLORLISTBOX: UINT = 0x0134;
pub const WM_CTLCOLORBTN: UINT = 0x0135;
/// State of a row a control wants drawn.
pub const ODS_SELECTED: u32 = 0x0001;
pub const ODS_DISABLED: u32 = 0x0004;
pub const ODS_FOCUS: u32 = 0x0010;

/// Window messages and commands used by minimise handling.
pub const WM_SYSCOMMAND: UINT = 0x0112;
pub const SC_MINIMIZE: WPARAM = 0xF020;
pub const SC_RESTORE: WPARAM = 0xF120;
pub const SC_CLOSE: WPARAM = 0xF060;
pub const WM_GETMINMAXINFO: UINT = 0x0024;
pub const SIZE_MINIMIZED: WPARAM = 1;
pub const GWL_STYLE: i32 = -16;
pub const SW_MINIMIZE: i32 = 6;
pub const SW_SHOWNOACTIVATE: i32 = 4;

/// Shell icons and folder picker.
pub const SHGFI_ICON: UINT = 0x0000_0100;
/// The path handed to `SHGetFileInfoW` is a PIDL and not a file name.
pub const SHGFI_PIDL: UINT = 0x0000_0008;
/// Apartment a window thread lives in (`COINIT_APARTMENTTHREADED`).
pub const COINIT_APARTMENTTHREADED: DWORD = 0x2;
pub const SHGFI_SMALLICON: UINT = 0x0000_0001;
pub const SHGFI_LARGEICON: UINT = 0x0000_0000;
pub const BIF_RETURNONLYFSDIRS: UINT = 0x0001;
pub const BIF_NEWDIALOGSTYLE: UINT = 0x0040;
pub const OFN_OVERWRITEPROMPT: UINT = 0x0002;
pub const OFN_PATHMUSTEXIST: UINT = 0x0800;
pub const OFN_EXPLORER: UINT = 0x0008_0000;
pub const MAX_PATH: usize = 260;

/// Balloon tips of the notification area icon.
pub const NIM_SETVERSION: DWORD = 0x0000_0004;
pub const NOTIFYICON_VERSION_4: UINT = 4;
pub const NIF_INFO: UINT = 0x0010;
pub const NIIF_INFO: UINT = 0x0000_0001;
pub const NIIF_NOSOUND: UINT = 0x0000_0010;

/// Process lookups that turn an application identifier into an icon.
pub const TH32CS_SNAPPROCESS: DWORD = 0x0000_0002;
pub const PROCESS_QUERY_LIMITED_INFORMATION: DWORD = 0x1000;
pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

/// System colours.
pub const COLOR_WINDOW: i32 = 5;
pub const COLOR_WINDOWTEXT: i32 = 8;
/// Background of a selected item (`COLOR_HIGHLIGHT`).
pub const COLOR_HIGHLIGHT: i32 = 13;
pub const COLOR_HIGHLIGHTTEXT: i32 = 14;
pub const COLOR_BTNFACE: i32 = 15;
/// A shaded edge - the colour a thin rule is drawn in.
pub const COLOR_3DSHADOW: i32 = 16;
pub const COLOR_GRAYTEXT: i32 = 17;
pub const COLOR_BTNTEXT: i32 = 18;
/// Light edge of a 3D element: the colour of a thin separator line.
pub const COLOR_3DLIGHT: i32 = 22;

/// Clipboard access.
pub const CF_UNICODETEXT: UINT = 13;
pub const GMEM_MOVEABLE: UINT = 0x0002;
/// Text drawn without a background (used for static controls on a coloured form).
pub const TRANSPARENT: i32 = 1;

// ----------------------------------------------------------------- extra imports
#[link(name = "user32")]
extern "system" {
    pub fn SystemParametersInfoW(action: UINT, param: UINT, data: *mut c_void, flags: UINT)
        -> BOOL;
    pub fn CreateFontIndirectW(log_font: *const LOGFONTW) -> HFONT;
    pub fn GetDpiForWindow(hwnd: HWND) -> UINT;
    pub fn SetProcessDpiAwarenessContext(context: HANDLE) -> BOOL;
    pub fn SetProcessDPIAware() -> BOOL;
    pub fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn ScreenToClient(hwnd: HWND, point: *mut POINT) -> BOOL;
    pub fn SetWindowPos(
        hwnd: HWND,
        insert_after: HWND,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: UINT,
    ) -> BOOL;
    pub fn IsIconic(hwnd: HWND) -> BOOL;
    pub fn GetSystemMetrics(index: i32) -> i32;
    pub fn GetSysColor(index: i32) -> DWORD;
    pub fn FindWindowW(class_name: LPCWSTR, window_name: LPCWSTR) -> HWND;
    pub fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
    pub fn DestroyIcon(icon: HICON) -> BOOL;
    pub fn GetParent(hwnd: HWND) -> HWND;
    pub fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    pub fn GetNextDlgTabItem(dialog: HWND, control: HWND, previous: BOOL) -> HWND;
    pub fn IsWindow(hwnd: HWND) -> BOOL;
    pub fn GetMenu(hwnd: HWND) -> HMENU;
    pub fn CheckMenuItem(menu: HMENU, id: UINT, check: UINT) -> DWORD;
    pub fn SetMenuDefaultItem(menu: HMENU, item: UINT, by_position: UINT) -> BOOL;
    pub fn CheckMenuRadioItem(
        menu: HMENU,
        first: UINT,
        last: UINT,
        check: UINT,
        flags: UINT,
    ) -> BOOL;
    pub fn GetKeyState(key: i32) -> i16;
    pub fn GetFocus() -> HWND;
    /// Identifier the control was created with (`GetDlgCtrlID`).
    pub fn GetDlgCtrlID(hwnd: HWND) -> i32;
    pub fn GetClassNameW(hwnd: HWND, buffer: LPWSTR, count: i32) -> i32;
    /// Stock brush of a system colour - unlike `CreateSolidBrush` it needs no
    /// cleanup, which matters for a brush that is handed out on every paint.
    pub fn GetSysColorBrush(index: i32) -> HBRUSH;
    pub fn FillRect(dc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    pub fn SetCursor(cursor: HCURSOR) -> HCURSOR;
    pub fn IsZoomed(hwnd: HWND) -> BOOL;
    pub fn IsWindowVisible(hwnd: HWND) -> BOOL;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn DeleteObject(object: HGDIOBJ) -> BOOL;
    pub fn SelectObject(dc: HDC, object: HGDIOBJ) -> HGDIOBJ;
    pub fn GetTextExtentPoint32W(dc: HDC, text: LPCWSTR, count: i32, size: *mut SIZE) -> BOOL;
}

// Draw a dashed rectangle around the focused row, the way Windows does.
#[link(name = "user32")]
extern "system" {
    pub fn DrawFocusRect(dc: HDC, rect: *const RECT) -> BOOL;
}

/// `LVS_OWNERDRAWFIXED`: the report does not draw its rows; it asks the window
/// (`WM_MEASUREITEM` for the height, `WM_DRAWITEM` for every row).
pub const LVS_OWNERDRAWFIXED: UINT = 0x0400;

pub const SS_OWNERDRAW: UINT = 0x000D;

// Dark menus, dark scroll bars and the dark variant of a control's theme.
//
// `SetPreferredAppMode` and `AllowDarkModeForWindow` have no documented names, so
// they are reached through the ordinals `uxtheme.dll` exports them with; on a
// system that does not know them the request is simply ignored.  Both the library
// and the symbols are resolved at runtime: the GNU toolchain has no import library
// for `uxtheme` (see `Module`).

/// Attribute number of the dark title bar; the older builds used `19`.
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
pub const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: u32 = 19;

/// Ordinals in `uxtheme.dll` (they are what the system's own windows use).
const UXTHEME_ALLOW_DARK_MODE_FOR_WINDOW: usize = 133;
const UXTHEME_SET_PREFERRED_APP_MODE: usize = 135;

/// Ask the system for a dark application look (menus, scroll bars, tooltips).
///
/// Returns false when the system does not know the call - on such a system the
/// program paints its own windows dark and leaves the menus as the system draws
/// them, which is the honest answer to "the system has no dark mode".
pub fn set_preferred_app_mode(dark: bool) -> bool {
    // `ForceDark` = 2, `ForceLight` = 3.
    call_uxtheme_ordinal(
        UXTHEME_SET_PREFERRED_APP_MODE,
        if dark { 2 } else { 3 },
        std::ptr::null_mut(),
    )
}

/// Let one window take part in the dark look of the theme engine.
fn allow_dark_mode_for_window(hwnd: HWND, dark: bool) {
    call_uxtheme_ordinal(
        UXTHEME_ALLOW_DARK_MODE_FOR_WINDOW,
        hwnd as usize,
        (if dark { 1 } else { 0 }) as *mut c_void,
    );
}

/// Call an undocumented `uxtheme` entry point; false when it is not there.
fn call_uxtheme_ordinal(ordinal: usize, first: usize, second: *mut c_void) -> bool {
    let Some(module) = uxtheme() else {
        return false;
    };
    let address = unsafe { GetProcAddress(module.handle, ordinal as *const i8) };
    if address.is_null() {
        return false;
    }
    let call: unsafe extern "system" fn(usize, *mut c_void) -> usize =
        unsafe { std::mem::transmute(address) };
    unsafe { call(first, second) };
    true
}

/// Ask for a dark title bar and dark borders (`dwmapi.dll`, loaded on demand).
pub fn dark_title_bar(hwnd: HWND, dark: bool) {
    if hwnd.is_null() {
        return;
    }
    type SetAttribute = unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32;
    static MODULE: std::sync::OnceLock<Option<Module>> = std::sync::OnceLock::new();
    let module = MODULE.get_or_init(|| Module::load("dwmapi.dll"));
    let Some(module) = module else {
        return;
    };
    let symbol = unsafe { module.symbol("DwmSetWindowAttribute") };
    if symbol.is_null() {
        return;
    }
    let function: SetAttribute = unsafe { std::mem::transmute(symbol) };
    let value: i32 = if dark { 1 } else { 0 };
    let size = std::mem::size_of::<i32>() as u32;
    unsafe {
        let value = &value as *const i32 as *const c_void;
        if function(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, value, size) != 0 {
            // Windows before 20H1 knows the same thing under the old number.
            function(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1, value, size);
        }
        // A window that is already on screen keeps its old frame until it is told
        // that the frame changed: without this nudge the title bar stays light until
        // the window is resized or moved.
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// The look of one control: `"DarkMode_Explorer"` makes the themed parts of a
/// control dark where the system supports it.
pub fn theme_control(hwnd: HWND, dark: bool) {
    if hwnd.is_null() {
        return;
    }
    if dark {
        allow_dark_mode_for_window(hwnd, true);
    }
    apply_theme(hwnd, if dark { "DarkMode_Explorer" } else { "Explorer" });
}

/// Size of a text in a given font (`cx` width, `cy` height), or zero when it
/// cannot be measured.
pub fn text_size(hwnd: HWND, font: HFONT, text: &str) -> SIZE {
    let text = text.trim();
    if text.is_empty() || hwnd.is_null() {
        return SIZE::default();
    }
    let buffer = wide(text);
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return SIZE::default();
        }
        let previous = if font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(dc, font as HGDIOBJ)
        };
        let mut size = SIZE::default();
        let measured =
            GetTextExtentPoint32W(dc, buffer.as_ptr(), buffer.len() as i32 - 1, &mut size);
        if !previous.is_null() {
            SelectObject(dc, previous);
        }
        ReleaseDC(hwnd, dc);
        if measured == 0 {
            SIZE::default()
        } else {
            size
        }
    }
}

#[link(name = "comctl32")]
extern "system" {
    pub fn ImageList_Create(
        width: i32,
        height: i32,
        flags: UINT,
        initial: i32,
        grow: i32,
    ) -> HIMAGELIST;
    pub fn ImageList_ReplaceIcon(list: HIMAGELIST, index: i32, icon: HICON) -> i32;
    /// Draw one image of the list into a device context; a row that paints itself
    /// needs this instead of letting the control draw the icon.
    pub fn ImageList_Draw(
        list: HIMAGELIST,
        index: i32,
        dc: HDC,
        x: i32,
        y: i32,
        flags: UINT,
    ) -> BOOL;
    /// Removes one image; `-1` removes all of them.
    ///
    /// This is the real entry point: `ImageList_RemoveAll` looks like a function
    /// in the headers but is only a macro, and `comctl32` does not export it, so
    /// calling it by that name fails at link time.
    pub fn ImageList_Remove(list: HIMAGELIST, index: i32) -> BOOL;
    pub fn ImageList_Destroy(list: HIMAGELIST) -> BOOL;
}

#[link(name = "shell32")]
extern "system" {
    pub fn SHGetFileInfoW(
        path: LPCWSTR,
        attributes: u32,
        info: *mut SHFILEINFOW,
        info_size: UINT,
        flags: UINT,
    ) -> usize;
    pub fn ExtractIconExW(
        file: LPCWSTR,
        index: i32,
        large: *mut HICON,
        small: *mut HICON,
        count: UINT,
    ) -> UINT;
    /// The icon of a file the way the shell itself resolves it.
    pub fn SHDefExtractIconW(
        file: LPCWSTR,
        index: i32,
        flags: UINT,
        large: *mut HICON,
        small: *mut HICON,
        size: DWORD,
    ) -> i32;
    pub fn SHBrowseForFolderW(info: *mut BROWSEINFOW) -> *mut c_void;
    pub fn SHGetPathFromIDListW(item: *const c_void, path: LPWSTR) -> BOOL;
    /// Item behind a shell parsing name (`shell:AppsFolder\...`) as a PIDL.
    pub fn SHParseDisplayName(
        name: LPCWSTR,
        bind_context: *mut c_void,
        item: *mut *mut c_void,
        attributes_in: u32,
        attributes_out: *mut u32,
    ) -> i32;
}

#[link(name = "comdlg32")]
extern "system" {
    pub fn GetSaveFileNameW(dialog: *mut OPENFILENAMEW) -> BOOL;
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoTaskMemFree(memory: *mut c_void);
    /// Enter an apartment; `models::COINIT_APARTMENTTHREADED` is the one a window
    /// thread wants.
    pub fn CoInitializeEx(reserved: *mut c_void, model: DWORD) -> i32;
}

#[link(name = "user32")]
extern "system" {
    pub fn OpenClipboard(owner: HWND) -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn CloseClipboard() -> BOOL;
    pub fn SetClipboardData(format: UINT, data: HANDLE) -> HANDLE;
    pub fn SetTextColor(dc: HDC, colour: DWORD) -> DWORD;
    pub fn SetBkMode(dc: HDC, mode: i32) -> i32;
    pub fn DrawTextW(
        dc: HDC,
        text: LPWSTR,
        count: i32,
        rect: *mut RECT,
        format: UINT,
    ) -> i32;
    pub fn GetDC(hwnd: HWND) -> HDC;
    pub fn ReleaseDC(hwnd: HWND, dc: HDC) -> i32;
    pub fn AdjustWindowRectEx(rect: *mut RECT, style: UINT, menu: BOOL, ex_style: UINT) -> BOOL;
}

/// Text drawing flags of `DrawTextW`.
pub const DT_WORDBREAK: UINT = 0x0010;
pub const DT_NOPREFIX: UINT = 0x0800;
/// One line, centred in the rectangle: what a list row of a report view uses.
pub const DT_SINGLELINE: UINT = 0x0020;
pub const DT_VCENTER: UINT = 0x0004;
/// What does not fit ends in an ellipsis instead of being cut off.
pub const DT_END_ELLIPSIS: UINT = 0x8000;
/// Draw an image of an image list over what is already there.
pub const ILD_TRANSPARENT: UINT = 0x0001;
/// Measure only: the rectangle is filled in and nothing is drawn.
pub const DT_CALCRECT: UINT = 0x0400;

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateSolidBrush(colour: DWORD) -> HBRUSH;
    pub fn SetBkColor(dc: HDC, colour: DWORD) -> DWORD;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn QueryFullProcessImageNameW(
        process: HANDLE,
        flags: u32,
        path: LPWSTR,
        size: *mut u32,
    ) -> BOOL;
    pub fn OpenProcess(access: DWORD, inherit: BOOL, process_id: DWORD) -> HANDLE;
    pub fn CloseHandle(handle: HANDLE) -> BOOL;
    pub fn GetApplicationUserModelId(process: HANDLE, length: *mut u32, aumid: LPWSTR) -> i32;
    pub fn CreateToolhelp32Snapshot(flags: DWORD, process_id: DWORD) -> HANDLE;
    pub fn Process32FirstW(snapshot: HANDLE, entry: *mut PROCESSENTRY32W) -> BOOL;
    pub fn Process32NextW(snapshot: HANDLE, entry: *mut PROCESSENTRY32W) -> BOOL;
    pub fn GetModuleFileNameW(module: HMODULE, path: LPWSTR, size: DWORD) -> DWORD;
    /// Expand `%SystemRoot%` and the other variables the environment defines.
    pub fn ExpandEnvironmentStringsW(source: LPCWSTR, target: LPWSTR, size: DWORD) -> DWORD;
    pub fn CreateActCtxW(context: *mut ACTCTXW) -> HANDLE;
    pub fn ActivateActCtx(context: HANDLE, cookie: *mut usize) -> BOOL;
    pub fn DeactivateActCtx(flags: u32, cookie: usize) -> BOOL;
    pub fn ReleaseActCtx(context: HANDLE);
    pub fn CreateMutexW(attributes: *mut c_void, initial_owner: BOOL, name: LPCWSTR) -> HANDLE;
    pub fn GlobalAlloc(flags: UINT, bytes: usize) -> HANDLE;
    pub fn GlobalLock(memory: HANDLE) -> *mut c_void;
    pub fn GlobalUnlock(memory: HANDLE) -> BOOL;
    pub fn GlobalFree(memory: HANDLE) -> HANDLE;
    pub fn LoadLibraryW(name: LPCWSTR) -> HMODULE;
    pub fn GetProcAddress(module: HMODULE, name: *const i8) -> *mut c_void;
    pub fn FreeLibrary(module: HMODULE) -> BOOL;
}

/// A library opened on first use.
///
/// `shlwapi`, `uxtheme` and `gdiplus` have no import library in the GNU
/// toolchain, and an import library would also fix the requirement at link time.
/// Loading them at runtime - exactly like `winsqlite3.dll` in `sqlite_ffi` -
/// keeps the build free of external tools and lets the application degrade
/// gracefully when a library is missing.
pub struct Module {
    handle: HMODULE,
}

impl Module {
    /// Open a DLL by name (`"shlwapi.dll"`).
    pub fn load(name: &str) -> Option<Module> {
        let name_w = wide(name);
        let handle = unsafe { LoadLibraryW(name_w.as_ptr()) };
        (!handle.is_null()).then_some(Module { handle })
    }

    /// Address of an exported function.
    ///
    /// # Safety
    /// The caller must know the real signature of the symbol; a wrong signature
    /// would call into the library with a mismatched stack.
    pub unsafe fn symbol(&self, name: &str) -> *mut c_void {
        match std::ffi::CString::new(name) {
            Ok(text) => GetProcAddress(self.handle, text.as_ptr()),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.handle);
        }
    }
}

// A DLL handle is a process-wide, immutable value: sharing it between threads is
// safe, and the library itself stays loaded until the last `Module` is dropped.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

/// `shlwapi.dll`, loaded once.
fn shlwapi() -> Option<&'static Module> {
    static MODULE: std::sync::OnceLock<Option<Module>> = std::sync::OnceLock::new();
    MODULE.get_or_init(|| Module::load("shlwapi.dll")).as_ref()
}

/// `uxtheme.dll`, loaded once.
fn uxtheme() -> Option<&'static Module> {
    static MODULE: std::sync::OnceLock<Option<Module>> = std::sync::OnceLock::new();
    MODULE.get_or_init(|| Module::load("uxtheme.dll")).as_ref()
}

// -------------------------------------------------------------------- new helpers
/// Per monitor aware (v2) DPI context, available since Windows 10 1703.
pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: HANDLE = -4isize as HANDLE;

/// The font Windows uses for dialog text (`lfMessageFont`), typically Segoe UI 9.
pub fn message_font_log() -> Option<LOGFONTW> {
    let mut metrics = NONCLIENTMETRICSW::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            std::mem::size_of::<NONCLIENTMETRICSW>() as UINT,
            &mut metrics as *mut NONCLIENTMETRICSW as *mut c_void,
            0,
        )
    };
    (ok != 0).then_some(metrics.lf_message_font)
}

/// Create a font from a description; `None` when GDI refuses.
pub fn create_font(log_font: &LOGFONTW) -> Option<HFONT> {
    let font = unsafe { CreateFontIndirectW(log_font) };
    (!font.is_null()).then_some(font)
}

/// The window font of the current system (falls back to the stock GUI font).
///
/// Using the system font is what makes the window look like a native Windows
/// application instead of a bitmap-font console: the same face, size and DPI as
/// Explorer or the settings dialogs.
pub fn system_font() -> HFONT {
    match message_font_log().and_then(|log| create_font(&log)) {
        Some(font) => font,
        None => unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT },
    }
}

/// System font enlarged (or reduced) by a percentage.
///
/// Users with tired eyes need a bigger interface; scaling the *system* font
/// keeps the look native while giving the text room.  `percent` is clamped to a
/// sensible range so a hand edited settings file cannot ask for an unreadable UI.
pub fn system_font_scaled(percent: u32) -> HFONT {
    let percent = percent.clamp(50, 300);
    if percent == 100 {
        return system_font();
    }
    let Some(mut log) = message_font_log() else {
        return system_font();
    };
    let height = log.lf_height;
    if height == 0 {
        return system_font();
    }
    // `lf_height` is negative for character height; keep the sign.
    let scaled = ((height as i64 * percent as i64) / 100) as i32;
    log.lf_height = if scaled == 0 { height } else { scaled };
    create_font(&log).unwrap_or_else(system_font)
}

/// Bold variant of the scaled system font (section headings, default buttons).
pub fn bold_font_scaled(percent: u32) -> HFONT {
    let mut log = message_font_log().unwrap_or_default();
    if log.lf_height != 0 {
        log.lf_height = ((log.lf_height as i64 * percent.clamp(50, 300) as i64) / 100) as i32;
    }
    log.lf_weight = 700; // FW_BOLD
    create_font(&log).unwrap_or_else(system_font)
}

/// Width of a text in a given font, in pixels (`0` when it cannot be measured).
///
/// The layout of the toolbars asks for the real width of a caption instead of
/// trusting a fixed number per button: a longer word than expected then makes the
/// row wrap, while a short one does not leave a hole next to it.
pub fn text_width(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    text_size(hwnd, font, text).cx.max(0)
}

/// Scale a value written for 96 DPI to the given DPI.
pub fn scale(value: i32, dpi: u32) -> i32 {
    if dpi == 0 || dpi == 96 {
        return value;
    }
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

/// DPI of the monitor a window is displayed on (96 when unknown).
pub fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// Opt into per monitor DPI awareness so the window is crisp on scaled displays.
pub fn make_dpi_aware() -> bool {
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
            return true;
        }
        SetProcessDPIAware() != 0
    }
}

/// Activate a side-by-side manifest, which is what loads common controls v6.
///
/// Returns `false` when the manifest is missing or invalid; the application then
/// keeps the classic look instead of failing.
pub fn activate_manifest(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    let source = wide(&path.display().to_string());
    let mut context = ACTCTXW {
        lp_source: source.as_ptr(),
        ..Default::default()
    };
    unsafe {
        let handle = CreateActCtxW(&mut context);
        if handle == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut cookie: usize = 0;
        let activated = ActivateActCtx(handle, &mut cookie) != 0;
        if !activated {
            ReleaseActCtx(handle);
        }
        activated
    }
}

/// Ask the theme engine to draw a control like its Explorer counterpart.
pub fn apply_theme(hwnd: HWND, name: &str) {
    type SetTheme = unsafe extern "system" fn(HWND, LPCWSTR, LPCWSTR) -> i32;
    let Some(module) = uxtheme() else {
        return;
    };
    let symbol = unsafe { module.symbol("SetWindowTheme") };
    if symbol.is_null() {
        return;
    }
    let function: SetTheme = unsafe { std::mem::transmute(symbol) };
    let name_w = wide(name);
    unsafe { function(hwnd, name_w.as_ptr(), std::ptr::null()) };
}

/// Resolve an indirect string such as `@{dll?resource}` or `ms-resource:`.
pub fn indirect_string(value: &str) -> Option<String> {
    type LoadIndirect = unsafe extern "system" fn(LPCWSTR, LPWSTR, UINT, *mut *mut c_void) -> i32;
    let module = shlwapi()?;
    let symbol = unsafe { module.symbol("SHLoadIndirectString") };
    if symbol.is_null() {
        return None;
    }
    let function: LoadIndirect = unsafe { std::mem::transmute(symbol) };
    let source = wide(value);
    let mut buffer = vec![0u16; 1024];
    let status = unsafe {
        function(
            source.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as UINT,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return None;
    }
    let text = read_wide(&buffer);
    (!text.trim().is_empty()).then_some(text)
}

/// Icon the shell associates with a file or an executable.
pub fn shell_icon(path: &str, small: bool) -> Option<HICON> {
    let path_w = wide(path);
    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_ICON
        | if small {
            SHGFI_SMALLICON
        } else {
            SHGFI_LARGEICON
        };
    let result = unsafe {
        SHGetFileInfoW(
            path_w.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as UINT,
            flags,
        )
    };
    (result != 0 && !info.h_icon.is_null()).then_some(info.h_icon)
}

/// Icon `index` of an `.exe`/`.dll`/`.ico` file.
///
/// A registry `IconUri` names a file *and* the icon inside it
/// (`shell32.dll,-101`).  A negative index is the resource identifier of the icon
/// and not its position in the file, which is exactly what the API expects: with a
/// negative index it starts at the icon whose identifier is the absolute value.
pub fn extract_icon_at(path: &str, index: i32, small: bool) -> Option<HICON> {
    let path_w = wide(path);
    let mut large: HICON = std::ptr::null_mut();
    let mut small_icon: HICON = std::ptr::null_mut();
    let count = unsafe { ExtractIconExW(path_w.as_ptr(), index, &mut large, &mut small_icon, 1) };
    if count == 0 || count == u32::MAX {
        return None;
    }
    let wanted = if small { small_icon } else { large };
    let unwanted = if small { large } else { small_icon };
    unsafe {
        if !unwanted.is_null() {
            DestroyIcon(unwanted);
        }
    }
    (!wanted.is_null()).then_some(wanted)
}

/// First icon of an `.exe`/`.dll`/`.ico` file.
pub fn extract_icon(path: &str, small: bool) -> Option<HICON> {
    extract_icon_at(path, 0, small)
}

/// Make sure the shell can answer on this thread.
///
/// The question "which icon does this application have" is answered by a shell
/// object, and a shell object needs COM.  The call is made once per process and
/// its answer is ignored on purpose: `S_FALSE` means the thread already runs in the
/// same apartment and `RPC_E_CHANGED_MODE` means it runs in another one - in both
/// cases somebody was quicker, and the lookup simply finds nothing.
pub fn ensure_com() {
    static STARTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    STARTED.get_or_init(|| unsafe {
        const S_OK: i32 = 0;
        const S_FALSE: i32 = 1;
        const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
        let status = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        status == S_OK || status == S_FALSE || status == RPC_E_CHANGED_MODE
    });
}

/// Icon of an application of the start menu (`shell:AppsFolder\<aumid>`).
///
/// This is the picture Windows itself shows next to an application, and both a
/// packaged application and a program that registered an application identifier
/// are reachable that way: the shell does the lookup, so the program here does not
/// have to repeat every rule of the start menu.
///
/// The name has to be turned into a PIDL first: `SHGetFileInfoW` reads a *file
/// name* and finds nothing under a `shell:` path, while it answers with the icon
/// when it is handed the item itself.
pub fn apps_folder_icon(aumid: &str, small: bool) -> Option<HICON> {
    let aumid = aumid.trim();
    if aumid.is_empty() {
        return None;
    }
    ensure_com();
    let parsing_name = wide(&format!("shell:AppsFolder\\{aumid}"));
    let mut item: *mut c_void = std::ptr::null_mut();
    let mut attributes: u32 = 0;
    let status = unsafe {
        SHParseDisplayName(
            parsing_name.as_ptr(),
            std::ptr::null_mut(),
            &mut item,
            0,
            &mut attributes,
        )
    };
    if status != 0 || item.is_null() {
        return None;
    }
    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_PIDL
        | SHGFI_ICON
        | if small {
            SHGFI_SMALLICON
        } else {
            SHGFI_LARGEICON
        };
    let result = unsafe {
        SHGetFileInfoW(
            item as LPCWSTR,
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as UINT,
            flags,
        )
    };
    unsafe { CoTaskMemFree(item) };
    (result != 0 && !info.h_icon.is_null()).then_some(info.h_icon)
}

/// Expand `%SystemRoot%` and the variables of the environment.
///
/// Registry values are stored unexpanded, and the icon of an installed program is
/// regularly written as `%SystemRoot%\System32\...`.
pub fn expand_environment(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let source = wide(text);
    let mut buffer = vec![0u16; 2048];
    let length = unsafe {
        ExpandEnvironmentStringsW(source.as_ptr(), buffer.as_mut_ptr(), buffer.len() as DWORD)
    };
    if length == 0 || length as usize > buffer.len() {
        return text.to_string();
    }
    read_wide(&buffer[..(length as usize).min(buffer.len())])
}

/// Free an icon created by [`shell_icon`] or [`extract_icon`].
pub fn destroy_icon(icon: HICON) {
    if !icon.is_null() {
        unsafe {
            DestroyIcon(icon);
        }
    }
}

/// `"Caption\0Pattern\0...\0\0"` as the file dialog expects it.
fn filter_string(filters: &[(&str, &str)]) -> Vec<u16> {
    let mut text: Vec<u16> = Vec::new();
    for (caption, pattern) in filters {
        text.extend(caption.encode_utf16());
        text.push(0);
        text.extend(pattern.encode_utf16());
        text.push(0);
    }
    text.push(0);
    text
}

/// "Save as" dialog; returns the chosen path.
pub fn save_file_name(
    owner: HWND,
    title: &str,
    filters: &[(&str, &str)],
    default_ext: &str,
    suggested: &str,
) -> Option<std::path::PathBuf> {
    let filter = filter_string(filters);
    let title_w = wide(title);
    let extension_w = wide(default_ext);
    let mut file = vec![0u16; 1024];
    fill_wide(&mut file, suggested);
    let mut dialog = OPENFILENAMEW {
        hwnd_owner: owner,
        lpstr_filter: filter.as_ptr(),
        n_filter_index: 1,
        lpstr_file: file.as_mut_ptr(),
        n_max_file: file.len() as u32,
        lpstr_title: title_w.as_ptr(),
        lpstr_def_ext: extension_w.as_ptr(),
        flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_EXPLORER,
        ..Default::default()
    };
    let accepted = unsafe { GetSaveFileNameW(&mut dialog) };
    if accepted == 0 {
        return None;
    }
    let path = read_wide(&file);
    (!path.trim().is_empty()).then(|| std::path::PathBuf::from(path))
}

/// Folder picker; returns the chosen directory.
pub fn pick_folder(owner: HWND, title: &str) -> Option<std::path::PathBuf> {
    let title_w = wide(title);
    let mut display = vec![0u16; MAX_PATH];
    let mut info = BROWSEINFOW {
        hwnd_owner: owner,
        psz_display_name: display.as_mut_ptr(),
        lpsz_title: title_w.as_ptr(),
        ul_flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let item = unsafe { SHBrowseForFolderW(&mut info) };
    if item.is_null() {
        return None;
    }
    let mut path = vec![0u16; MAX_PATH + 1];
    let ok = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) };
    unsafe { CoTaskMemFree(item) };
    if ok == 0 {
        return None;
    }
    let text = read_wide(&path);
    (!text.trim().is_empty()).then(|| std::path::PathBuf::from(text))
}

/// Full path of the running executable.
pub fn executable_path() -> std::path::PathBuf {
    let mut buffer = vec![0u16; 1024];
    let written = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buffer.as_mut_ptr(), 1024) };
    if written == 0 {
        return std::path::PathBuf::new();
    }
    std::path::PathBuf::from(read_wide(&buffer[..written as usize]))
}

/// Folder the executable lives in.
pub fn executable_directory() -> std::path::PathBuf {
    executable_path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default()
}

/// Put text on the clipboard (the *Копировать* buttons).
///
/// The clipboard owns the memory afterwards, which is why the block is handed
/// over with `SetClipboardData` instead of being copied and freed here.
pub fn copy_to_clipboard(owner: HWND, text: &str) -> bool {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let bytes = units.len() * 2;
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if memory.is_null() {
            return false;
        }
        let target = GlobalLock(memory) as *mut u16;
        if target.is_null() {
            GlobalFree(memory);
            return false;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
        GlobalUnlock(memory);

        if OpenClipboard(owner) == 0 {
            GlobalFree(memory);
            return false;
        }
        EmptyClipboard();
        let stored = SetClipboardData(CF_UNICODETEXT, memory);
        if stored.is_null() {
            GlobalFree(memory);
        }
        CloseClipboard();
        !stored.is_null()
    }
}

/// Colour used by the system for secondary text.
pub fn grey_text_colour() -> DWORD {
    unsafe { GetSysColor(COLOR_GRAYTEXT) }
}

/// Colour of a normal window background.
pub fn window_colour() -> DWORD {
    unsafe { GetSysColor(COLOR_WINDOW) }
}

/// Message box with the application name as its caption.
pub fn message_box(hwnd: HWND, text: &str, kind: UINT) {
    let text_w = wide(text);
    let caption = wide(crate::appname::APPLICATION_TITLE);
    unsafe {
        MessageBoxW(hwnd, text_w.as_ptr(), caption.as_ptr(), MB_OK | kind);
    }
}

// ------------------------------------------------------------- z-order of a page
/// Puts a window at the top of the z-order (used for page contents).
pub const HWND_TOP: HWND = 0isize as HWND;
/// Puts a window at the bottom of the z-order (used for a tab frame).
pub const HWND_BOTTOM: HWND = 1isize as HWND;
pub const SWP_NOSIZE: UINT = 0x0001;
pub const SWP_NOMOVE: UINT = 0x0002;
pub const SWP_NOZORDER: UINT = 0x0004;
pub const SWP_NOACTIVATE: UINT = 0x0010;
/// The frame (title bar and borders) has to be redrawn: what a window with a new
/// dark or light frame is told (`dark_title_bar`).
pub const SWP_FRAMECHANGED: UINT = 0x0020;

/// Move a window in the z-order without touching its position or size.
pub fn set_z_order(window: HWND, after: HWND) {
    unsafe {
        SetWindowPos(
            window,
            after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

// ------------------------------------------------ tooltips, popups, decorations
/// Class of the standard tooltip control.
pub const TOOLTIPS_CLASS: &str = "tooltips_class32";
pub const TTS_ALWAYSTIP: UINT = 0x0000_0001;
pub const TTS_NOPREFIX: UINT = 0x0000_0002;
pub const WS_POPUP: UINT = 0x8000_0000;
pub const TTF_IDISHWND: UINT = 0x0001;
pub const TTF_SUBCLASS: UINT = 0x0010;
pub const TTM_SETDELAYTIME: UINT = WM_USER + 3;
pub const TTM_SETMAXTIPWIDTH: UINT = WM_USER + 24;
pub const TTM_ADDTOOLW: UINT = WM_USER + 50;
pub const TTM_UPDATETIPTEXTW: UINT = WM_USER + 57;
pub const TTDT_INITIAL: i32 = 0;
pub const TTDT_AUTOPOP: i32 = 2;
/// `WM_CONTEXTMENU` is also what the keyboard (Shift+F10, menu key) produces.
pub const TOOLTIP_HINT_DELAY: i32 = 0;

/// The control a tooltip describes: `TOOLINFOW` of the common controls.
#[repr(C)]
pub struct TOOLINFOW {
    pub cb_size: UINT,
    pub u_flags: UINT,
    pub hwnd: HWND,
    pub u_id: usize,
    pub rect: RECT,
    pub h_inst: HINSTANCE,
    pub lpsz_text: LPCWSTR,
    pub l_param: LPARAM,
}

impl Default for TOOLINFOW {
    fn default() -> Self {
        TOOLINFOW {
            cb_size: std::mem::size_of::<TOOLINFOW>() as UINT,
            u_flags: 0,
            hwnd: std::ptr::null_mut(),
            u_id: 0,
            rect: RECT::default(),
            h_inst: std::ptr::null_mut(),
            lpsz_text: std::ptr::null(),
            l_param: 0,
        }
    }
}

/// Create a tooltip window for a parent window; `None` when it cannot be made.
pub fn create_tooltip(parent: HWND) -> Option<HWND> {
    let class = wide(TOOLTIPS_CLASS);
    let tooltip = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            module_handle(),
            std::ptr::null_mut(),
        )
    };
    if tooltip.is_null() {
        return None;
    }
    unsafe {
        // Wrap long explanations instead of drawing one endless line.
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, 0, 420);
        SendMessageW(tooltip, TTM_SETDELAYTIME, TTDT_INITIAL as WPARAM, 700);
        SendMessageW(tooltip, TTM_SETDELAYTIME, TTDT_AUTOPOP as WPARAM, 12_000);
    }
    Some(tooltip)
}

/// Attach a tooltip text to a control; the control stays the tool owner.
pub fn tooltip_attach(tooltip: HWND, control: HWND, text: &str) {
    if tooltip.is_null() || control.is_null() {
        return;
    }
    let text_w = wide(text);
    let mut info = TOOLINFOW {
        u_flags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: control,
        u_id: control as usize,
        lpsz_text: text_w.as_ptr(),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tooltip,
            TTM_ADDTOOLW,
            0,
            &mut info as *mut TOOLINFOW as LPARAM,
        );
    }
}

/// Replace the text of a tooltip attached earlier.
pub fn tooltip_set_text(tooltip: HWND, control: HWND, text: &str) {
    if tooltip.is_null() || control.is_null() {
        return;
    }
    let text_w = wide(text);
    let mut info = TOOLINFOW {
        u_flags: TTF_IDISHWND,
        hwnd: control,
        u_id: control as usize,
        lpsz_text: text_w.as_ptr(),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tooltip,
            TTM_UPDATETIPTEXTW,
            0,
            &mut info as *mut TOOLINFOW as LPARAM,
        );
    }
}

/// One entry of a popup menu; an empty text inserts a separator.
pub struct MenuItem {
    pub id: usize,
    pub text: String,
    pub enabled: bool,
}

impl MenuItem {
    /// A normal entry.
    pub fn new(id: usize, text: &str) -> MenuItem {
        MenuItem {
            id,
            text: text.to_string(),
            enabled: true,
        }
    }

    /// A separator (its id is never returned).
    pub fn separator() -> MenuItem {
        MenuItem {
            id: 0,
            text: String::new(),
            enabled: false,
        }
    }

    /// An entry that is visible but unavailable (greyed).
    pub fn disabled(id: usize, text: &str) -> MenuItem {
        MenuItem {
            id,
            text: text.to_string(),
            enabled: false,
        }
    }
}

/// Show a popup menu at a screen position and return the chosen id.
///
/// `default_id` is highlighted (the double-click equivalent of the menu);
/// `x`/`y` of `-1` mean "where the mouse is", which is what the keyboard
/// invocation of a context menu needs.
pub fn popup_menu(
    owner: HWND,
    items: &[MenuItem],
    x: i32,
    y: i32,
    default_id: Option<usize>,
) -> Option<usize> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for item in items {
            if item.text.is_empty() {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                continue;
            }
            let flags = MF_STRING | if item.enabled { 0 } else { MF_GRAYED };
            let text_w = wide(&item.text);
            AppendMenuW(menu, flags, item.id, text_w.as_ptr());
        }
        if let Some(default) = default_id {
            SetMenuDefaultItem(menu, default as UINT, 0);
        }
        let mut point = POINT { x, y };
        if x < 0 || y < 0 {
            GetCursorPos(&mut point);
        }
        SetForegroundWindow(owner);
        let command = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            owner,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        (command != 0).then_some(command as usize)
    }
}

/// Open a file, folder or address with the program Windows associates with it.
pub fn shell_open(hwnd: HWND, target: &str) -> bool {
    let operation = wide("open");
    let path = wide(target);
    let result = unsafe {
        ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    result as isize > 32
}

// ------------------------------------------------------------- tab control
/// Class of the standard tab control.
pub const TAB_CLASS: &str = "SysTabControl32";
pub const TCS_MULTILINE: UINT = 0x0200;
pub const TCM_FIRST: UINT = 0x1300;
pub const TCM_INSERTITEMW: UINT = TCM_FIRST + 62;
pub const TCM_GETCURSEL: UINT = TCM_FIRST + 11;
pub const TCM_SETCURSEL: UINT = TCM_FIRST + 12;
/// Number of tabs in the control.
pub const TCM_GETITEMCOUNT: UINT = TCM_FIRST + 4;
/// Screen rectangle of one tab (`TCM_GETITEMRECT`).
pub const TCM_GETITEMRECT: UINT = TCM_FIRST + 10;
/// Turn a window rectangle into the page rectangle (Windows gives the two
/// directions of this message an opposite meaning, so the application measures
/// the tab itself instead of trusting it).
pub const TCM_ADJUSTRECT: UINT = TCM_FIRST + 40;
pub const TCN_FIRST: UINT = -550isize as UINT;
pub const TCN_SELCHANGE: UINT = TCN_FIRST - 1;
pub const TCIF_TEXT: UINT = 0x0001;

/// `TCITEMW` of the tab control.
#[repr(C)]
pub struct TCITEMW {
    pub mask: UINT,
    pub dw_state: DWORD,
    pub dw_state_mask: DWORD,
    pub psz_text: LPWSTR,
    pub cch_text_max: i32,
    pub i_image: i32,
    pub l_param: LPARAM,
}

impl Default for TCITEMW {
    fn default() -> Self {
        TCITEMW {
            mask: 0,
            dw_state: 0,
            dw_state_mask: 0,
            psz_text: std::ptr::null_mut(),
            cch_text_max: 0,
            i_image: 0,
            l_param: 0,
        }
    }
}

/// Add one page to a tab control (at the end).
///
/// The index is passed explicitly: the control does *not* accept the usual
/// "append" value `-1` through a 64 bit message parameter, which is what made
/// the very first version of this window show an empty tab strip.
pub fn tab_insert(tab: HWND, title: &str) -> bool {
    if tab.is_null() {
        return false;
    }
    let index = unsafe { SendMessageW(tab, TCM_GETITEMCOUNT, 0, 0) } as usize;
    let mut text = wide(title);
    let mut item = TCITEMW {
        mask: TCIF_TEXT,
        psz_text: text.as_mut_ptr(),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tab,
            TCM_INSERTITEMW,
            index,
            &mut item as *mut TCITEMW as LPARAM,
        ) >= 0
    }
}

/// Height the tab strip of a tab control needs (rows plus its frame).
///
/// The control answers `TCM_GETITEMRECT` in its own coordinates, which is the
/// only measurement that follows the font and the DPI without guessing.
pub fn tab_strip_height(tab: HWND, fallback: i32) -> i32 {
    if tab.is_null() {
        return fallback;
    }
    let mut rect = RECT::default();
    let answered =
        unsafe { SendMessageW(tab, TCM_GETITEMRECT, 0, &mut rect as *mut RECT as LPARAM) };
    if answered == 0 || rect.bottom <= 0 {
        return fallback;
    }
    // The strip sits under the control's frame, so a couple of pixels are added.
    rect.bottom + 3
}

/// Reads one tab back (`TCITEMW` of `TCM_GETITEMW`).
pub const TCM_GETITEMW: UINT = TCM_FIRST + 60;

/// Currently selected page of a tab control.
pub fn tab_selected(tab: HWND) -> usize {
    unsafe { SendMessageW(tab, TCM_GETCURSEL, 0, 0) as usize }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The Win32 structures passed by pointer must match the system ABI exactly;
    /// a wrong layout would corrupt memory instead of failing loudly.
    ///
    /// The message queue, the notification area and the list view live in one
    /// test: they are the same question asked of three structures, and splitting
    /// them only made the answer harder to find.
    #[test]
    fn structures_match_the_win64_abi() {
        if size_of::<usize>() != 8 {
            return;
        }
        // HWND, UINT, WPARAM, LPARAM, DWORD, POINT, DWORD (see tagMSG).
        assert_eq!(size_of::<MSG>(), 48);
        assert_eq!(offset_of!(MSG, w_param), 16);
        assert_eq!(offset_of!(MSG, l_param), 24);
        assert_eq!(offset_of!(MSG, time), 32);
        assert_eq!(offset_of!(MSG, pt), 36);
        assert_eq!(offset_of!(MSG, l_private), 44);

        // NOTIFYICONDATAW as defined by Vista and later.
        assert_eq!(size_of::<NOTIFYICONDATAW>(), 976);
        assert_eq!(offset_of!(NOTIFYICONDATAW, h_icon), 32);
        assert_eq!(offset_of!(NOTIFYICONDATAW, sz_tip), 40);
        assert_eq!(offset_of!(NOTIFYICONDATAW, sz_info), 304);
        assert_eq!(offset_of!(NOTIFYICONDATAW, sz_info_title), 820);
        assert_eq!(offset_of!(NOTIFYICONDATAW, guid_item), 952);
        assert_eq!(offset_of!(NOTIFYICONDATAW, h_balloon_icon), 968);

        assert_eq!(size_of::<LVITEMW>(), 88);
        assert_eq!(offset_of!(LVITEMW, psz_text), 24);
        assert_eq!(offset_of!(LVITEMW, l_param), 40);
        assert_eq!(offset_of!(LVITEMW, i_group), 80);

        assert_eq!(size_of::<LVCOLUMNW>(), 56);
        assert_eq!(offset_of!(LVCOLUMNW, psz_text), 16);
        assert_eq!(offset_of!(LVCOLUMNW, i_sub_item), 28);

        assert_eq!(size_of::<NMHDR>(), 24);
        assert_eq!(size_of::<NMLISTVIEW>(), 64);
        assert_eq!(offset_of!(NMLISTVIEW, i_item), 24);
        assert_eq!(offset_of!(NMLISTVIEW, l_param), 56);

        // Every structure with a size field fills it in by itself.
        assert_eq!(
            NOTIFYICONDATAW::default().cb_size as usize,
            size_of::<NOTIFYICONDATAW>()
        );
    }

    #[test]
    fn text_helpers_round_trip() {
        let mut buffer = [0u16; 8];
        fill_wide(&mut buffer, "уведомление");
        assert!(read_wide(&buffer).starts_with("уведом"));
        fill_wide(&mut buffer, "ok");
        assert_eq!(read_wide(&buffer), "ok");
        assert_eq!(wide("ab"), vec![97, 98, 0]);
    }

    /// The structures added for the friendlier window must match the system ABI
    /// as well: a single wrong field and Windows writes across our memory.
    #[test]
    fn presentation_layout_matches_win64() {
        if size_of::<usize>() != 8 {
            return;
        }
        // LOGFONTW: 5 LONGs (20) + 8 BYTEs (28) + WCHAR[32] (64) = 92.
        assert_eq!(size_of::<LOGFONTW>(), 92);
        assert_eq!(offset_of!(LOGFONTW, lf_face_name), 28);
        // NONCLIENTMETRICSW: cbSize + 5 ints + 5 fonts + 4 ints + iPaddedBorderWidth.
        assert_eq!(size_of::<NONCLIENTMETRICSW>(), 504);
        assert_eq!(offset_of!(NONCLIENTMETRICSW, lf_message_font), 408);
        assert_eq!(size_of::<ACTCTXW>(), 64);
        assert_eq!(offset_of!(ACTCTXW, lp_assembly_directory), 24);
        assert_eq!(size_of::<PROCESSENTRY32W>(), 568);
        assert_eq!(offset_of!(PROCESSENTRY32W, sz_exe_file), 44);
        assert_eq!(size_of::<SHFILEINFOW>(), 696);
        assert_eq!(offset_of!(SHFILEINFOW, sz_display_name), 16);
        assert_eq!(size_of::<OPENFILENAMEW>(), 152);
        assert_eq!(offset_of!(OPENFILENAMEW, lpstr_file), 48);
        assert_eq!(offset_of!(OPENFILENAMEW, flags), 96);
        assert_eq!(size_of::<BROWSEINFOW>(), 64);
        assert_eq!(size_of::<MINMAXINFO>(), 40);
        assert_eq!(size_of::<SIZE>(), 8);
        // The two messages a control that paints itself sends carry these; a wrong
        // layout here would write into the wrong fields of the caller's structure.
        assert_eq!(size_of::<MEASUREITEMSTRUCT>(), 32);
        assert_eq!(offset_of!(MEASUREITEMSTRUCT, item_height), 16);
        assert_eq!(offset_of!(MEASUREITEMSTRUCT, item_data), 24);
        assert_eq!(size_of::<DRAWITEMSTRUCT>(), 64);
        assert_eq!(offset_of!(DRAWITEMSTRUCT, hwnd_item), 24);
        assert_eq!(offset_of!(DRAWITEMSTRUCT, hdc), 32);
        assert_eq!(offset_of!(DRAWITEMSTRUCT, rc_item), 40);
        assert_eq!(offset_of!(DRAWITEMSTRUCT, item_data), 56);
        assert_eq!(size_of::<GDIPLUS_STARTUP_INPUT>(), 24);

        // A structure with a size field has to fill it in by itself.
        assert_eq!(
            NONCLIENTMETRICSW::default().cb_size as usize,
            size_of::<NONCLIENTMETRICSW>()
        );
        assert_eq!(ACTCTXW::default().cb_size as usize, size_of::<ACTCTXW>());
        assert_eq!(
            PROCESSENTRY32W::default().dw_size as usize,
            size_of::<PROCESSENTRY32W>()
        );
        assert_eq!(
            OPENFILENAMEW::default().l_struct_size as usize,
            size_of::<OPENFILENAMEW>()
        );
        assert_eq!(GDIPLUS_STARTUP_INPUT::default().version, 1);
    }

    #[test]
    fn dpi_scaling_keeps_96_as_identity() {
        assert_eq!(scale(0, 96), 0);
        assert_eq!(scale(26, 96), 26);
        assert_eq!(scale(26, 0), 26);
        // 150 %: the row height grows with the monitor.
        assert_eq!(scale(26, 144), 39);
        assert_eq!(scale(100, 120), 125);
        assert_eq!(scale(8, 192), 16);
    }

    #[test]
    fn file_dialog_filter_is_double_terminated() {
        let filter = filter_string(&[("CSV", "*.csv"), ("Все файлы", "*.*")]);
        assert_eq!(filter.last(), Some(&0));
        assert_eq!(filter[filter.len() - 2], 0);
        let text: Vec<u16> = "CSV\0*.csv\0Все файлы\0*.*\0\0".encode_utf16().collect();
        assert_eq!(filter, text);
    }

    /// Control messages cannot be checked by the compiler: a wrong number is
    /// accepted silently and only shows up as a cosmetic defect (or worse).
    /// These values are cross-checked against the Windows SDK.
    #[test]
    fn a_tab_control_accepts_its_items() {
        // The tab control is a real window class: the test creates one and adds
        // two pages, which is the only way to be sure the hand written messages
        // and the `TCITEMW` layout match what Windows expects.
        let instance = module_handle();
        let class = wide("STATIC");
        let empty = wide("");
        let parent = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                empty.as_ptr(),
                WS_POPUP,
                0,
                0,
                320,
                240,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        };
        if parent.is_null() {
            // A session without a window station (a service, a headless runner):
            // nothing to test, and nothing to fail either.
            return;
        }
        let tab_class = wide(TAB_CLASS);
        let tab = unsafe {
            CreateWindowExW(
                0,
                tab_class.as_ptr(),
                empty.as_ptr(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                300,
                200,
                parent,
                1 as HMENU,
                instance,
                std::ptr::null_mut(),
            )
        };
        assert!(!tab.is_null(), "the tab control was not created");
        assert!(tab_insert(tab, "Основное"), "первая вкладка не добавилась");
        assert!(tab_insert(tab, "Полностью"), "вторая вкладка не добавилась");
        let count = unsafe { SendMessageW(tab, TCM_GETITEMCOUNT, 0, 0) };
        assert_eq!(count, 2, "the control kept both pages");
        assert_eq!(tab_selected(tab), 0, "the first page is selected");
        assert!(
            tab_strip_height(tab, 0) > 0,
            "the strip has a height to reserve"
        );
        // The title of the first tab has to come back as it was given.
        let mut buffer = vec![0u16; 64];
        let mut item = TCITEMW {
            mask: TCIF_TEXT,
            psz_text: buffer.as_mut_ptr(),
            cch_text_max: buffer.len() as i32,
            ..Default::default()
        };
        let answered =
            unsafe { SendMessageW(tab, TCM_GETITEMW, 0, &mut item as *mut TCITEMW as LPARAM) };
        assert_eq!(answered, 1, "the tab control answered the item");
        let title: String =
            String::from_utf16_lossy(&buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(0)]);
        assert_eq!(title, "Основное", "текст вкладки читается");
        unsafe {
            DestroyWindow(parent);
        }
    }
}

