//! Collector for the documented WinRT API `UserNotificationListener` (port of
//! `sources/uns.py`).
//!
//! `Windows.UI.Notifications.Management.UserNotificationListener` (Windows 10
//! 1607+) is the only *documented* way for a third-party process to read the
//! notification center.  The Python implementation reaches it through the
//! optional `winsdk` projection; the native build has no dependencies, so the
//! interfaces are declared here by hand and the runtime is entered through the
//! `Ro*` entry points of `combase.dll` (resolved at run time with
//! `LoadLibraryW`, exactly like `sqlite_ffi` takes SQLite from `winsqlite3.dll`).
//!
//! Everything below that cannot be guessed was read out of the metadata Windows
//! ships in `C:\Windows\System32\WinMetadata\*.winmd` with
//! `tools/winmd-dump.py` (interface IIDs and the order of the methods - which is
//! the order of the function pointers in the COM vtable) and cross-checked
//! against the MIDL generated SDK header `windows.ui.notifications.management.h`:
//! both give the same IIDs and the same order for `IUserNotificationListener`
//! and its statics interface.  Two facts are easy to get wrong:
//!
//! * a `requires` interface is *not* part of the vtable (the generated header
//!   spells it out: `IVectorView_impl : IInspectable /* requires IIterable<T> */`),
//!   so `IAsyncOperation<T>` has `put_Completed`/`get_Completed`/`GetResults` at
//!   slots 6/7/8 and `IAsyncInfo` is asked for with `QueryInterface`;
//! * `Windows.Foundation.DateTime` counts 100 ns steps since 1601, the same unit
//!   as the `FILETIME` values in the notification database, so the `time` helpers
//!   apply unchanged.
//!
//! What the API exposes (measured on Windows 11 24H2): the notification id, its
//! creation and expiration time, the text lines of the `ToastGeneric` binding,
//! the binding template and language - but **not** images, actions, tags or the
//! raw XML, which stay the business of `wpndb`.  `UserNotification.AppInfo`
//! returns `E_NOTIMPL` on Windows 11, so the application is taken from the
//! notification database by notification id instead (`Notification.Id` there *is*
//! `UserNotification.Id`, see `docs/ARCHITECTURE.md`); without that fallback the
//! record would not merge with the `wpndb` observation of the same notification.
//!
//! Errors never stop the collector: an unavailable API, a denied permission or a
//! failing read is reported in the state and the `wpndb` source keeps working.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_void};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::appname::NameResolver;
use crate::model::NotificationRecord;
use crate::store::Journal;
use crate::win32::{self, GUID};
use crate::{time, wpndb};

pub const SOURCE: &str = "uns";
/// The API is polled at most this often; the collector loop may run more often.
pub const DEFAULT_POLL_SECONDS: f64 = 1.5;
/// How many notifications are remembered as "already read".
const SEEN_LIMIT: usize = 2048;
/// Failures in a row after which the source switches itself off for a while.
const MAX_READ_ERRORS: u32 = 20;
/// Cooldown after such a run of failures.
const RETRY_PAUSE_SECONDS: u64 = 300;
/// How long an asynchronous WinRT call may take before it is given up on.
const ASYNC_TIMEOUT: Duration = Duration::from_secs(20);

/// `NotificationKinds.Toast` (the only kind this collector reads).
const NOTIFICATION_KINDS_TOAST: i32 = 1;
/// `UserNotificationListenerAccessStatus.Allowed` / `.Denied`.
const ACCESS_ALLOWED: i32 = 1;
const ACCESS_DENIED: i32 = 2;

/// `RO_INIT_MULTITHREADED`: the runtime class is marked `MTA` and `Agile`.
const RO_INIT_MULTITHREADED: u32 = 1;
/// `RPC_E_CHANGED_MODE`: the thread was already initialised for another model.
const RPC_E_CHANGED_MODE: i32 = -2_147_417_338; // 0x80010106

/// Name of the activatable runtime class.
const LISTENER_CLASS: &str = "Windows.UI.Notifications.Management.UserNotificationListener";
/// Template of the binding that carries the visible text lines.
const TOAST_GENERIC: &str = "ToastGeneric";

// ---------------------------------------------------------------- identifiers
// GUIDs exactly as the WinMD `GuidAttribute` values carry them; a runtime check
// (`the_objects_implement_the_interfaces_used_here`) compares them with the
// interfaces the objects really implement.
const IID_ACTIVATION_FACTORY: GUID = GUID {
    data1: 0x0000_0035,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_ASYNC_INFO: GUID = GUID {
    data1: 0x0000_0036,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_USER_NOTIFICATION_LISTENER_STATICS: GUID = GUID {
    data1: 0xff61_23cf,
    data2: 0x4386,
    data3: 0x4aa3,
    data4: [0xb7, 0x3d, 0xb8, 0x04, 0xe5, 0xb6, 0x3b, 0x23],
};
const IID_USER_NOTIFICATION_LISTENER: GUID = GUID {
    data1: 0x6255_3e41,
    data2: 0x8a06,
    data3: 0x4cef,
    data4: [0x82, 0x15, 0x60, 0x33, 0xa5, 0xbe, 0x4b, 0x03],
};
const IID_USER_NOTIFICATION: GUID = GUID {
    data1: 0xadf7_e52f,
    data2: 0x4e53,
    data3: 0x42d5,
    data4: [0x9c, 0x33, 0xeb, 0x5e, 0xa5, 0x15, 0xb2, 0x3e],
};
const IID_NOTIFICATION: GUID = GUID {
    data1: 0x1080_37fe,
    data2: 0xeb76,
    data3: 0x4f82,
    data4: [0x97, 0xbc, 0xda, 0x07, 0x53, 0x0a, 0x2e, 0x20],
};
const IID_NOTIFICATION_VISUAL: GUID = GUID {
    data1: 0x6883_5b8e,
    data2: 0xaa56,
    data3: 0x4e11,
    data4: [0x86, 0xd3, 0x5f, 0x9a, 0x69, 0x57, 0xbc, 0x5b],
};
const IID_NOTIFICATION_BINDING: GUID = GUID {
    data1: 0xf29e_4b85,
    data2: 0x0370,
    data3: 0x4ad3,
    data4: [0xb4, 0xea, 0xda, 0x9e, 0x35, 0xe7, 0xea, 0xbf],
};
const IID_ADAPTIVE_NOTIFICATION_TEXT: GUID = GUID {
    data1: 0x46d4_a3be,
    data2: 0x609a,
    data3: 0x4326,
    data4: [0xa4, 0x0b, 0xbf, 0xde, 0x87, 0x20, 0x34, 0xa3],
};
const IID_APP_INFO: GUID = GUID {
    data1: 0xcf7f_59b3,
    data2: 0x6a09,
    data3: 0x4de8,
    data4: [0xa6, 0xc0, 0x57, 0x92, 0xd5, 0x68, 0x80, 0xd1],
};
const IID_APP_DISPLAY_INFO: GUID = GUID {
    data1: 0x1aeb_1103,
    data2: 0xe4d4,
    data3: 0x41aa,
    data4: [0xa4, 0xf6, 0xc4, 0xa2, 0x76, 0xe7, 0x9e, 0xac],
};

// --------------------------------------------------------------------- vtables
/// `IInspectable` prefix of every WinRT interface plus the declared methods.
///
/// The slots are written in the order the metadata declares them, because that
/// order *is* the ABI: each field is a function pointer and the field after
/// `get_trust_level` is slot 6, the first method of the interface itself.
macro_rules! com_vtbl {
    ($name:ident { $( $slot:ident : $ty:ty ),* $(,)? }) => {
        #[repr(C)]
        pub struct $name {
            pub query_interface:
                unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
            pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
            pub release: unsafe extern "system" fn(*mut c_void) -> u32,
            pub get_iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> i32,
            pub get_runtime_class_name:
                unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
            pub get_trust_level: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
            $( pub $slot: $ty, )*
        }
    };
}

com_vtbl!(ListenerStaticsVtbl {
    get_current: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
});

com_vtbl!(ListenerVtbl {
    request_access_async: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_access_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    add_notification_changed: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> i32,
    remove_notification_changed: unsafe extern "system" fn(*mut c_void, i64) -> i32,
    get_notifications_async: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32,
    get_notification: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32,
    clear_notifications: unsafe extern "system" fn(*mut c_void) -> i32,
    remove_notification: unsafe extern "system" fn(*mut c_void, u32) -> i32,
});

com_vtbl!(AsyncInfoVtbl {
    get_id: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    get_error_code: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    cancel: unsafe extern "system" fn(*mut c_void) -> i32,
    close: unsafe extern "system" fn(*mut c_void) -> i32,
});

com_vtbl!(AsyncOperationVtbl {
    put_completed: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_completed: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_results: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
});

com_vtbl!(VectorViewVtbl {
    get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    index_of: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u8, *mut u32) -> i32,
    get_many: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void, *mut u32) -> i32,
});

com_vtbl!(UserNotificationVtbl {
    get_notification: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_app_info: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_id: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_creation_time: unsafe extern "system" fn(*mut c_void, *mut DateTime) -> i32,
});

com_vtbl!(NotificationVtbl {
    get_expiration_time: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_expiration_time: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_visual: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_visual: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
});

com_vtbl!(NotificationVisualVtbl {
    get_language: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_language: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_bindings: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_binding: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32,
});

com_vtbl!(NotificationBindingVtbl {
    get_template: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_template: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_language: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_language: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_hints: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_text_elements: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
});

com_vtbl!(AdaptiveNotificationTextVtbl {
    get_text: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_text: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_language: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    put_language: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
});

com_vtbl!(AppInfoVtbl {
    get_id: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_app_user_model_id: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_display_info: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_package_family_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
});

com_vtbl!(AppDisplayInfoVtbl {
    get_display_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_description: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_logo: unsafe extern "system" fn(*mut c_void, *mut Size, *mut *mut c_void) -> i32,
});

com_vtbl!(ReferenceVtbl {
    get_value: unsafe extern "system" fn(*mut c_void, *mut DateTime) -> i32,
});

/// `Windows.Foundation.DateTime`: 100 ns ticks since 1601-01-01, i.e. the same
/// unit as the `FILETIME` values in the notification database.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateTime {
    pub universal_time: i64,
}

/// `Windows.Foundation.Size` (only needed to call `IAppDisplayInfo::GetLogo`,
/// which this collector does not use - the images come from `wpndb`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

// The `IInspectable` prefix alone: enough to call `QueryInterface`, `AddRef`,
// `Release` and `GetIids` on *any* WinRT interface.
com_vtbl!(InspectableVtbl {});

// -------------------------------------------------------------------- runtime
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

type RoInitializeFn = unsafe extern "system" fn(u32) -> i32;
type RoGetActivationFactoryFn =
    unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32;
type CreateStringFn = unsafe extern "system" fn(*const u16, u32, *mut *mut c_void) -> i32;
type DeleteStringFn = unsafe extern "system" fn(*mut c_void) -> i32;
type GetStringRawBufferFn = unsafe extern "system" fn(*mut c_void, *mut u32) -> *const u16;

/// Entry points of `combase.dll` the collector uses.
///
/// The library is loaded at run time (like `winsqlite3.dll` in `sqlite_ffi`), so
/// nothing has to be linked against the Windows SDK: `RoInitialize`,
/// `RoGetActivationFactory` and the `HSTRING` helpers all live there.
struct Api {
    initialize: RoInitializeFn,
    get_activation_factory: RoGetActivationFactoryFn,
    create_string: CreateStringFn,
    delete_string: DeleteStringFn,
    get_string_raw_buffer: GetStringRawBufferFn,
}

static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();

/// The loaded `combase.dll` entry points, or the reason they are missing.
fn api() -> Result<&'static Api, String> {
    let entry = API.get_or_init(|| unsafe {
        let module = LoadLibraryW(win32::wide("combase.dll").as_ptr());
        if module.is_null() {
            return None;
        }
        macro_rules! entry {
            ($name:literal, $type:ty) => {
                std::mem::transmute::<*mut c_void, $type>(GetProcAddress(
                    module,
                    concat!($name, "\0").as_ptr() as *const c_char,
                ))
            };
        }
        Some(Api {
            initialize: entry!("RoInitialize", RoInitializeFn),
            get_activation_factory: entry!("RoGetActivationFactory", RoGetActivationFactoryFn),
            create_string: entry!("WindowsCreateString", CreateStringFn),
            delete_string: entry!("WindowsDeleteString", DeleteStringFn),
            get_string_raw_buffer: entry!("WindowsGetStringRawBuffer", GetStringRawBufferFn),
        })
    });
    entry
        .as_ref()
        .ok_or_else(|| "библиотека WinRT (combase.dll) недоступна".to_string())
}

/// A counted string of the Windows Runtime (`HSTRING`).
struct Hstring(*mut c_void);

impl Hstring {
    fn new(text: &str) -> Result<Hstring, String> {
        let api = api()?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut value: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { (api.create_string)(wide.as_ptr(), wide.len() as u32, &mut value) };
        if hr < 0 {
            return Err(format!("не удалось создать строку WinRT (0x{:08x})", hr as u32));
        }
        Ok(Hstring(value))
    }

    fn text(&self) -> String {
        let Ok(api) = api() else {
            return String::new();
        };
        let mut length: u32 = 0;
        let pointer = unsafe { (api.get_string_raw_buffer)(self.0, &mut length) };
        if pointer.is_null() || length == 0 {
            return String::new();
        }
        let slice = unsafe { std::slice::from_raw_parts(pointer, length as usize) };
        String::from_utf16_lossy(slice)
    }
}

impl Drop for Hstring {
    fn drop(&mut self) {
        if let Ok(api) = api() {
            unsafe {
                (api.delete_string)(self.0);
            }
        }
    }
}

/// An interface pointer (any WinRT interface, which starts at `IInspectable`).
///
/// Ownership is single: `Release` runs when the value is dropped, `QueryInterface`
/// hands out a new counted pointer.  The objects of this API are marked `Agile`,
/// so a value may be used from any thread of the process that created it.
struct Com(*mut c_void);

impl Com {
    /// Same as [`Com::vtbl`], but the value is counted again so it can be used
    /// after the original pointer was dropped (`AddRef`).
    fn clone_ref(&self) -> Com {
        unsafe {
            (self.vtbl::<InspectableVtbl>().add_ref)(self.0);
        }
        Com(self.0)
    }

    /// Take ownership of a pointer the runtime wrote into an out parameter.
    ///
    /// # Safety
    /// `pointer` must be a counted interface pointer this module received and has
    /// not released yet.
    unsafe fn from_raw(pointer: *mut c_void) -> Option<Com> {
        (!pointer.is_null()).then_some(Com(pointer))
    }

    /// The vtable of the interface the pointer was obtained as.
    ///
    /// # Safety
    /// `T` must be the vtable that belongs to the interface `self` refers to.
    unsafe fn vtbl<T>(&self) -> &T {
        &**(self.0 as *const *const T)
    }

    /// `QueryInterface`: another interface of the same object.
    ///
    /// # Safety
    /// The object must be reachable through `self` (the pointer is live).
    unsafe fn query(&self, iid: &GUID) -> Result<Com, i32> {
        let mut value: *mut c_void = std::ptr::null_mut();
        let hr = (self.vtbl::<InspectableVtbl>().query_interface)(self.0, iid, &mut value);
        if hr < 0 {
            return Err(hr);
        }
        Ok(Com(value))
    }

    /// The interfaces the object implements (`IInspectable::GetIids`).
    ///
    /// The tests use this to check that the identifiers declared in this module
    /// really are the interfaces the runtime hands out.
    ///
    /// # Safety
    /// The object must be reachable through `self`.
    unsafe fn iids(&self) -> Vec<GUID> {
        let mut count: u32 = 0;
        let mut list: *mut GUID = std::ptr::null_mut();
        let hr = (self.vtbl::<InspectableVtbl>().get_iids)(self.0, &mut count, &mut list);
        if hr < 0 || list.is_null() {
            return Vec::new();
        }
        let result = std::slice::from_raw_parts(list, count as usize).to_vec();
        win32::CoTaskMemFree(list as *mut c_void);
        result
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                (self.vtbl::<InspectableVtbl>().release)(self.0);
            }
        }
    }
}

/// Ask the notification listener statics for the singleton listener.
///
/// `UserNotificationListener.Current` is a static property, so it lives on the
/// statics interface of the activation factory - the class itself cannot be
/// created directly (`activatableClassId` with no factory interface).
fn create_listener() -> Result<Com, String> {
    let api = api()?;
    let class = Hstring::new(LISTENER_CLASS)?;
    let mut factory: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        (api.get_activation_factory)(class.0, &IID_ACTIVATION_FACTORY, &mut factory)
    };
    // The class name is not needed any more, whether the call worked or not.
    if hr < 0 {
        return Err(format!(
            "класс WinRT {LISTENER_CLASS} недоступен (0x{:08x})",
            hr as u32
        ));
    }
    let factory = unsafe { Com::from_raw(factory) }
        .ok_or_else(|| "фабрика класса вернула пустой указатель".to_string())?;
    let statics = unsafe { factory.query(&IID_USER_NOTIFICATION_LISTENER_STATICS) }
        .map_err(|hr| {
            format!(
                "интерфейс статических методов недоступен (0x{:08x})",
                hr as u32
            )
        })?;
    let mut listener: *mut c_void = std::ptr::null_mut();
    let hr = unsafe { (statics.vtbl::<ListenerStaticsVtbl>().get_current)(statics.0, &mut listener) };
    if hr < 0 {
        return Err(format!("Current недоступен (0x{:08x})", hr as u32));
    }
    unsafe { Com::from_raw(listener) }
        .ok_or_else(|| "Current вернул пустой указатель".to_string())
}

// ------------------------------------------------------------------ read side
/// Shape of a property getter: `HRESULT get_X(this, out)`.
type GetterFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;

/// Read a string property; the value is copied out and then released.
///
/// # Safety
/// `getter` must be a getter of the interface `object` was obtained as.
unsafe fn string_property(object: *mut c_void, getter: GetterFn) -> Option<String> {
    let mut value: *mut c_void = std::ptr::null_mut();
    if getter(object, &mut value) < 0 || value.is_null() {
        return None;
    }
    // The value is an `HSTRING`, so it is freed with `WindowsDeleteString` and
    // never with `Release`: wrapping it in `Hstring` is what takes ownership.
    let text = Hstring(value).text();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Read an interface property (a counted pointer, or nothing when absent).
///
/// # Safety
/// `getter` must be a getter of the interface `object` was obtained as.
unsafe fn object_property(object: *mut c_void, getter: GetterFn) -> Option<Com> {
    let mut value: *mut c_void = std::ptr::null_mut();
    if getter(object, &mut value) < 0 {
        return None;
    }
    Com::from_raw(value)
}

/// Wait until an `IAsyncOperation<T>` has finished.
///
/// The operation is polled through `IAsyncInfo::get_Status`, which the object
/// implements as a *required* interface; the vtable of the operation itself only
/// carries `put_Completed`, `get_Completed` and `GetResults` (slots 6, 7, 8).
fn wait_for(operation: &Com) -> Result<(), String> {
    let info = unsafe { operation.query(&IID_ASYNC_INFO) }
        .map_err(|hr| format!("IAsyncInfo недоступен (0x{:08x})", hr as u32))?;
    let deadline = Instant::now() + ASYNC_TIMEOUT;
    loop {
        let mut status: i32 = 0;
        let hr = unsafe { (info.vtbl::<AsyncInfoVtbl>().get_status)(info.0, &mut status) };
        if hr < 0 {
            return Err(format!(
                "не удалось узнать состояние вызова (0x{:08x})",
                hr as u32
            ));
        }
        match status {
            1 => break, // Completed
            2 | 3 => {
                // Canceled or Error: the error code says which.
                let mut code: i32 = 0;
                unsafe {
                    (info.vtbl::<AsyncInfoVtbl>().get_error_code)(info.0, &mut code);
                }
                return Err(format!(
                    "асинхронный вызов WinRT не удался (0x{:08x})",
                    code as u32
                ));
            }
            _ => {
                if Instant::now() >= deadline {
                    return Err("асинхронный вызов WinRT не завершился вовремя".to_string());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    Ok(())
}

/// The result of an `IAsyncOperation<T>` where `T` is an object.
fn await_operation(operation: Com) -> Result<Com, String> {
    wait_for(&operation)?;
    let mut result: *mut c_void = std::ptr::null_mut();
    let hr =
        unsafe { (operation.vtbl::<AsyncOperationVtbl>().get_results)(operation.0, &mut result) };
    if hr < 0 {
        return Err(format!("результат вызова недоступен (0x{:08x})", hr as u32));
    }
    unsafe { Com::from_raw(result) }.ok_or_else(|| "вызов вернул пустой результат".to_string())
}

/// The result of an `IAsyncOperation<T>` where `T` is a value (an enumeration).
///
/// The ABI is the same call, only the type of the out parameter differs, so the
/// function pointer is re-interpreted instead of declaring a second vtable shape.
fn await_value(operation: Com) -> Result<i32, String> {
    wait_for(&operation)?;
    let getter: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 =
        unsafe { std::mem::transmute(operation.vtbl::<AsyncOperationVtbl>().get_results) };
    let mut value: i32 = -1;
    let hr = unsafe { getter(operation.0, &mut value) };
    if hr < 0 {
        return Err(format!("результат вызова недоступен (0x{:08x})", hr as u32));
    }
    Ok(value)
}

/// Everything the notification center currently shows, as WinRT objects.
fn read_notifications(listener: &Com) -> Result<Vec<Com>, String> {
    let mut operation: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        (listener.vtbl::<ListenerVtbl>().get_notifications_async)(
            listener.0,
            NOTIFICATION_KINDS_TOAST,
            &mut operation,
        )
    };
    if hr < 0 {
        return Err(format!(
            "GetNotificationsAsync не принят (0x{:08x})",
            hr as u32
        ));
    }
    let operation = unsafe { Com::from_raw(operation) }
        .ok_or_else(|| "GetNotificationsAsync вернул пустой указатель".to_string())?;
    let vector = await_operation(operation)?;
    let mut size: u32 = 0;
    let hr = unsafe { (vector.vtbl::<VectorViewVtbl>().get_size)(vector.0, &mut size) };
    if hr < 0 {
        return Err(format!("размер списка недоступен (0x{:08x})", hr as u32));
    }
    let mut items = Vec::with_capacity(size as usize);
    for index in 0..size {
        let mut item: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { (vector.vtbl::<VectorViewVtbl>().get_at)(vector.0, index, &mut item) };
        if hr < 0 {
            continue; // one broken entry must not stop the others
        }
        if let Some(com) = unsafe { Com::from_raw(item) } {
            items.push(com);
        }
    }
    Ok(items)
}

/// One `UserNotification`, read out into plain values.
///
/// Reading is separated from mapping on purpose: everything below this point is
/// pure Rust, so the mapping can be tested without a notification center.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    pub id: u32,
    /// 100 ns ticks since 1601 (a `FILETIME`), as Windows reports them.
    pub created_ticks: Option<i64>,
    pub expires_ticks: Option<i64>,
    /// Application user model id, when the platform reports it (`AppInfo`).
    pub aumid: Option<String>,
    pub app_name: Option<String>,
    /// `HRESULT` of `AppInfo` when it could not be read (`E_NOTIMPL` on Windows 11).
    pub app_info_error: Option<u32>,
    pub template: Option<String>,
    pub language: Option<String>,
    pub texts: Vec<String>,
}

/// Read one `UserNotification`.
fn observe(notification: &Com) -> Result<Observed, String> {
    let vtbl = unsafe { notification.vtbl::<UserNotificationVtbl>() };
    let mut id: u32 = 0;
    if unsafe { (vtbl.get_id)(notification.0, &mut id) } < 0 {
        return Err("не удалось прочитать Id уведомления".to_string());
    }
    let mut created = DateTime::default();
    let created_ticks = (unsafe { (vtbl.get_creation_time)(notification.0, &mut created) } >= 0)
        .then_some(created.universal_time)
        .filter(|ticks| *ticks > 0);
    let mut observed = Observed {
        id,
        created_ticks,
        ..Default::default()
    };

    let mut app_info: *mut c_void = std::ptr::null_mut();
    let hr = unsafe { (vtbl.get_app_info)(notification.0, &mut app_info) };
    if hr < 0 {
        observed.app_info_error = Some(hr as u32);
    } else if let Some(info) = unsafe { Com::from_raw(app_info) } {
        let info_vtbl = unsafe { info.vtbl::<AppInfoVtbl>() };
        observed.aumid = unsafe { string_property(info.0, info_vtbl.get_app_user_model_id) };
        if let Some(display) = unsafe { object_property(info.0, info_vtbl.get_display_info) } {
            let display_vtbl = unsafe { display.vtbl::<AppDisplayInfoVtbl>() };
            observed.app_name =
                unsafe { string_property(display.0, display_vtbl.get_display_name) };
        }
    }

    let (texts, template, language, expires) = read_visual(notification);
    observed.texts = texts;
    observed.template = template;
    observed.language = language;
    observed.expires_ticks = expires;
    Ok(observed)
}

/// Text lines of the `ToastGeneric` binding, its template, language and the
/// expiration time of the toast.
fn read_visual(
    notification: &Com,
) -> (Vec<String>, Option<String>, Option<String>, Option<i64>) {
    let mut texts = Vec::new();
    let mut template = None;
    let mut language = None;
    let mut expires = None;
    let toast_vtbl = unsafe { notification.vtbl::<UserNotificationVtbl>() };
    let Some(toast) = (unsafe { object_property(notification.0, toast_vtbl.get_notification) })
    else {
        return (texts, template, language, expires);
    };
    let toast_vtbl = unsafe { toast.vtbl::<NotificationVtbl>() };
    // `ExpirationTime` is an `IReference<DateTime>`: a boxed value that may be
    // absent, so both the pointer and the value inside it are checked.
    if let Some(expiration) =
        unsafe { object_property(toast.0, toast_vtbl.get_expiration_time) }
    {
        let mut value = DateTime::default();
        let hr = unsafe { (expiration.vtbl::<ReferenceVtbl>().get_value)(expiration.0, &mut value) };
        if hr >= 0 && value.universal_time > 0 {
            expires = Some(value.universal_time);
        }
    }
    let Some(visual) = (unsafe { object_property(toast.0, toast_vtbl.get_visual) }) else {
        return (texts, template, language, expires);
    };
    let Ok(name) = Hstring::new(TOAST_GENERIC) else {
        return (texts, template, language, expires);
    };
    let visual_vtbl = unsafe { visual.vtbl::<NotificationVisualVtbl>() };
    let mut binding: *mut c_void = std::ptr::null_mut();
    if unsafe { (visual_vtbl.get_binding)(visual.0, name.0, &mut binding) } < 0 {
        return (texts, template, language, expires);
    }
    let Some(binding) = (unsafe { Com::from_raw(binding) }) else {
        return (texts, template, language, expires);
    };
    let binding_vtbl = unsafe { binding.vtbl::<NotificationBindingVtbl>() };
    template = unsafe { string_property(binding.0, binding_vtbl.get_template) };
    language = unsafe { string_property(binding.0, binding_vtbl.get_language) };
    texts = read_texts(&binding);
    (texts, template, language, expires)
}

/// The text lines of a binding, in the order the sender wrote them.
fn read_texts(binding: &Com) -> Vec<String> {
    let binding_vtbl = unsafe { binding.vtbl::<NotificationBindingVtbl>() };
    let Some(elements) = (unsafe { object_property(binding.0, binding_vtbl.get_text_elements) })
    else {
        return Vec::new();
    };
    let mut size: u32 = 0;
    let hr = unsafe { (elements.vtbl::<VectorViewVtbl>().get_size)(elements.0, &mut size) };
    if hr < 0 {
        return Vec::new();
    }
    let mut texts = Vec::with_capacity(size as usize);
    for index in 0..size {
        let mut item: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { (elements.vtbl::<VectorViewVtbl>().get_at)(elements.0, index, &mut item) };
        if hr < 0 {
            continue;
        }
        let Some(element) = (unsafe { Com::from_raw(item) }) else {
            continue;
        };
        let element_vtbl = unsafe { element.vtbl::<AdaptiveNotificationTextVtbl>() };
        if let Some(text) = unsafe { string_property(element.0, element_vtbl.get_text) } {
            texts.push(text);
        }
    }
    texts
}

// ------------------------------------------------------------------- mapping
/// How the access status is called in the interface and in the log.
pub fn access_status_text(status: i32) -> &'static str {
    match status {
        ACCESS_ALLOWED => tr!("доступ разрешён"),
        ACCESS_DENIED => tr!("доступ запрещён пользователем"),
        0 => tr!("доступ ещё не определён"),
        _ => tr!("неизвестный ответ системы"),
    }
}

/// The `extra` and `raw` JSON of a record (the same facts in both places, as the
/// `wpndb` collector does it).
fn describe_observed(observed: &Observed, aumid: Option<&str>, app_name: Option<&str>) -> (serde_json::Value, serde_json::Value) {
    let app_info_error = observed.app_info_error.map(|code| format!("0x{code:08x}"));
    let extra = json!({
        "binding_template": observed.template,
        "binding_language": observed.language,
        // `null` while the platform reports the application itself; the code is
        // kept so the reason can be seen in the journal.
        "app_info_error": app_info_error,
    });
    let raw = json!({
        "notification_id": observed.id,
        "creation_time": observed.created_ticks,
        "expiration_time": observed.expires_ticks,
        "texts": observed.texts,
        "app_user_model_id": aumid,
        "display_name": app_name,
        "binding_template": observed.template,
        "binding_language": observed.language,
        "app_info_error": app_info_error,
        "parser": SOURCE,
        "collected_utc": time::now_iso(),
    });
    (extra, raw)
}

/// Turn what was read into a journal record.
///
/// `fallback_aumid` is the application from the notification database, used when
/// the platform does not report `AppInfo` (Windows 11 answers `E_NOTIMPL`).  The
/// notification id is the same in both places, which is what lets the two
/// observations merge into one journal row instead of producing a duplicate with
/// an unknown application.
pub fn build_record(
    observed: &Observed,
    fallback_aumid: Option<&str>,
    resolver: &NameResolver,
) -> NotificationRecord {
    let aumid = observed
        .aumid
        .clone()
        .or_else(|| fallback_aumid.map(str::to_string));
    let app_name = observed
        .app_name
        .clone()
        .or_else(|| aumid.as_deref().and_then(|id| resolver.resolve(id)));
    let mut lines = observed.texts.clone();
    let title = (!lines.is_empty()).then(|| lines.remove(0));
    let body = (!lines.is_empty()).then(|| lines.remove(0));
    let (extra, raw) = describe_observed(observed, aumid.as_deref(), app_name.as_deref());
    NotificationRecord {
        source: SOURCE.to_string(),
        aumid,
        app_name,
        notification_type: "toast".to_string(),
        windows_id: Some(i64::from(observed.id)),
        title,
        body,
        extra_text: lines,
        created_utc: observed.created_ticks.and_then(time::filetime_to_iso),
        expires_utc: observed.expires_ticks.and_then(time::filetime_to_iso),
        extra,
        raw,
        ..Default::default()
    }
}

// -------------------------------------------------------------------- polling
/// What one poll of the notification center reports to the collector status.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// `false` when the source does not read at all (API missing, access denied,
    /// switched off, paused after repeated failures).
    pub available: bool,
    /// How the source is doing: the access status, or the reason it is off.
    pub state: String,
    /// Notifications the notification center shows right now.
    pub visible: usize,
    /// Records handed to the journal during this poll.
    pub emitted: usize,
    /// True when the platform does not report the application (`E_NOTIMPL`).
    pub app_info_missing: bool,
    /// Records for which the application came from the notification database.
    pub fallback_used: u64,
    /// The reason of the last failure, when there was one.
    pub error: Option<String>,
}

impl Report {
    /// A report of a source that is not reading, with the reason in `state`.
    pub fn disabled(reason: impl Into<String>) -> Report {
        Report {
            available: false,
            state: reason.into(),
            visible: 0,
            emitted: 0,
            app_info_missing: false,
            fallback_used: 0,
            error: None,
        }
    }

    /// One line for the log and the status file.
    pub fn describe(&self) -> String {
        if !self.available {
            return tf!("не читается: {}", "cannot be read: {}", self.state);
        }
        if let Some(error) = &self.error {
            return tf!("ошибка чтения: {error}", "read error: {error}");
        }
        let mut text = tf!("{}  •  видно {}", "{}  •  visible {}", self.state, self.visible);
        if self.app_info_missing {
            text.push_str(tr!("  •  имя программы из базы Windows"));
        }
        text
    }

    /// JSON form for `status.json` (readable by `wnl-cli doctor` and scripts).
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "available": self.available,
            "state": self.state,
            "visible": self.visible,
            "emitted": self.emitted,
            "app_info_missing": self.app_info_missing,
            "fallback_used": self.fallback_used,
            "error": self.error,
        })
    }

    /// Read the JSON form back (missing fields become defaults).
    pub fn from_json(value: &serde_json::Value) -> Report {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        };
        Report {
            available: value
                .get("available")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            state: text("state"),
            visible: value
                .get("visible")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize,
            emitted: value
                .get("emitted")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize,
            app_info_missing: value
                .get("app_info_missing")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            fallback_used: value
                .get("fallback_used")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            error: value
                .get("error")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        }
    }
}

/// Why a poll failed: `fatal` means the source cannot work like this at all
/// (the API is missing or access was denied), anything else is a retry.
struct Failure {
    fatal: bool,
    message: String,
}

impl Failure {
    fn fatal(message: impl Into<String>) -> Failure {
        Failure {
            fatal: true,
            message: message.into(),
        }
    }

    fn retry(message: impl Into<String>) -> Failure {
        Failure {
            fatal: false,
            message: message.into(),
        }
    }
}

// -------------------------------------------------------------------- session
/// Per-thread state of the source: the listener, the access answer and what was
/// read before.
///
/// WinRT objects are usable from any thread here (the class is `Agile`), but a
/// listener, its access status and the "already read" cache belong to the thread
/// that uses them, so the state lives in a thread local: the collector thread
/// and the window of `wnl.exe` never disturb each other.
#[derive(Default)]
struct Session {
    /// `true` once this thread called `RoInitialize`.
    initialized: bool,
    listener: Option<Com>,
    /// The access answer of Windows (`UserNotificationListenerAccessStatus`).
    access: i32,
    /// The access request is made once per thread.
    access_asked: bool,
    /// Dedup key -> content hash of every notification written to the journal.
    seen: HashMap<String, String>,
    /// True when the platform answered `E_NOTIMPL` for `AppInfo`.
    app_info_missing: bool,
    /// How often the application had to come from the database.
    fallback_used: u64,
    /// Failures in a row and the pause that follows them.
    errors_in_a_row: u32,
    paused_until: Option<Instant>,
    /// Not more than one poll per this interval.
    last_poll: Option<Instant>,
    last_report: Option<Report>,
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// Read the notification center and store what is new.
///
/// Returns what happened instead of an error: a source that cannot read must
/// never stop the rest of the collection.
pub fn poll(
    database: Option<&wpndb::NotificationDatabase>,
    journal: &Journal,
    resolver: &NameResolver,
    poll_seconds: f64,
    duplicate_window_seconds: i64,
    rules: &crate::rules::RuleSet,
) -> Report {
    SESSION.with(|cell| {
        cell.borrow_mut().poll(
            database,
            journal,
            resolver,
            poll_seconds,
            duplicate_window_seconds,
            rules,
        )
    })
}

/// Ask Windows for the access status without reading anything.
///
/// Used by `wnl-cli doctor`: it answers "can this process read the notification
/// center" without touching the journal.
pub fn probe() -> Result<String, String> {
    SESSION.with(|cell| {
        let mut session = cell.borrow_mut();
        session.ensure()?;
        let listener = session
            .listener
            .as_ref()
            .ok_or_else(|| "слушатель уведомлений недоступен".to_string())?;
        let count = read_notifications(listener)?.len();
        Ok(tf!(
            "{}; видно {}",
            "{}; {} visible",
            access_status_text(session.access),
            count
        ))
    })
}

impl Session {
    /// One poll: the interval gate, the pause after failures and the report.
    fn poll(
        &mut self,
        database: Option<&wpndb::NotificationDatabase>,
        journal: &Journal,
        resolver: &NameResolver,
        poll_seconds: f64,
        duplicate_window_seconds: i64,
        rules: &crate::rules::RuleSet,
    ) -> Report {
        let now = Instant::now();
        if let Some(until) = self.paused_until {
            if now < until {
                return Report::disabled("пауза после ошибок чтения");
            }
            self.paused_until = None;
            self.errors_in_a_row = 0;
        }
        if let Some(last) = self.last_poll {
            // Reading more often than the configured interval would only repeat
            // work: the notification center changes at human speed.
            let interval = Duration::from_secs_f64(poll_seconds.clamp(0.2, 60.0));
            if now.duration_since(last) < interval {
                return self
                    .last_report
                    .clone()
                    .unwrap_or_else(|| Report::disabled("ещё не читался"));
            }
        }
        self.last_poll = Some(now);
        let report = match self.read_once(
            database,
            journal,
            resolver,
            duplicate_window_seconds,
            rules,
        ) {
            Ok(report) => {
                self.errors_in_a_row = 0;
                report
            }
            Err(failure) if failure.fatal => {
                // The API is missing or access was denied: there is nothing to
                // retry quickly, so the source stays quiet for a while.
                self.listener = None;
                self.access_asked = false;
                self.paused_until = Some(now + Duration::from_secs(RETRY_PAUSE_SECONDS));
                Report::disabled(failure.message)
            }
            Err(failure) => {
                self.errors_in_a_row += 1;
                if self.errors_in_a_row >= MAX_READ_ERRORS {
                    // A new listener is created on the next attempt.
                    self.listener = None;
                    self.paused_until = Some(now + Duration::from_secs(RETRY_PAUSE_SECONDS));
                }
                let mut report = Report::disabled(self.state_text());
                report.available = true;
                report.app_info_missing = self.app_info_missing;
                report.fallback_used = self.fallback_used;
                report.error = Some(failure.message);
                report
            }
        };
        self.last_report = Some(report.clone());
        report
    }

    /// How the source is called in the status line.
    fn state_text(&self) -> String {
        access_status_text(self.access).to_string()
    }

    /// Create the listener (once per thread) and check the access.
    fn ensure(&mut self) -> Result<(), String> {
        if self.listener.is_some() {
            return Ok(());
        }
        let api = api()?;
        if !self.initialized {
            // A thread that already lives in a single-threaded apartment (the
            // window of `wnl.exe` does) answers `RPC_E_CHANGED_MODE`; WinRT works
            // there as well, the runtime class is `Agile`.
            let hr = unsafe { (api.initialize)(RO_INIT_MULTITHREADED) };
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                return Err(format!("WinRT не запустилась (0x{:08x})", hr as u32));
            }
            self.initialized = true;
        }
        let listener = create_listener()?;
        let mut status: i32 = 0;
        let hr =
            unsafe { (listener.vtbl::<ListenerVtbl>().get_access_status)(listener.0, &mut status) };
        if hr >= 0 {
            self.access = status;
        }
        let mut request_error = None;
        if self.access != ACCESS_ALLOWED && !self.access_asked {
            self.access_asked = true;
            request_error = request_access(&listener).err();
        }
        if self.access != ACCESS_ALLOWED {
            let status = access_status_text(self.access);
            return Err(match request_error {
                Some(error) => format!(
                    "Windows не разрешила доступ к центру уведомлений ({status}), запрос не удался: {error}"
                ),
                None => format!("Windows не разрешила доступ к центру уведомлений ({status})"),
            });
        }
        self.listener = Some(listener);
        Ok(())
    }

    /// One read of the notification center together with the journal write.
    fn read_once(
        &mut self,
        database: Option<&wpndb::NotificationDatabase>,
        journal: &Journal,
        resolver: &NameResolver,
        duplicate_window_seconds: i64,
        rules: &crate::rules::RuleSet,
    ) -> Result<Report, Failure> {
        self.ensure().map_err(Failure::fatal)?;
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| Failure::fatal("слушатель уведомлений недоступен"))?
            .clone_ref();
        let notifications = read_notifications(&listener).map_err(Failure::retry)?;
        let mut records: Vec<NotificationRecord> = Vec::new();
        let mut fresh: Vec<(String, String)> = Vec::new();
        let mut current: HashSet<String> = HashSet::new();
        let mut visible = 0usize;
        let mut fallback_used = 0u64;
        for notification in &notifications {
            // One unreadable notification must not stop the others.
            let Ok(observed) = observe(notification) else {
                continue;
            };
            visible += 1;
            if observed.app_info_error.is_some() {
                self.app_info_missing = true;
            }
            // Windows 11 does not implement `AppInfo`, and the notification
            // database knows the same id, so the application comes from there.
            let fallback = if observed.aumid.is_none() {
                database.and_then(|db| db.aumid_for_id(i64::from(observed.id)).ok().flatten())
            } else {
                None
            };
            if fallback.is_some() {
                fallback_used += 1;
            }
            let record = build_record(&observed, fallback.as_deref(), resolver);
            let key = record.dedup_key();
            let hash = record.content_hash();
            current.insert(key.clone());
            if self.seen.get(&key) == Some(&hash) {
                continue; // already in the journal and unchanged
            }
            fresh.push((key, hash));
            if let (Some(aumid), Some(name)) = (&record.aumid, &record.app_name) {
                resolver.remember(aumid, name);
            }
            // Rules decide whether the notification reaches the journal; the
            // pop-ups stay the business of `wpndb`, otherwise a single
            // notification would raise two of them.
            if rules.evaluate(&record).is_skipped() {
                continue;
            }
            records.push(record);
        }
        if !records.is_empty() {
            journal
                .record_many(&records, duplicate_window_seconds)
                .map_err(|error| Failure::retry(error.to_string()))?;
            // The source keeps no cursor, so a pass without records has nothing
            // to remember: writing the bookkeeping anyway used to touch the
            // journal on every idle pass.
            journal
                .touch_source(SOURCE, None, records.len() as i64, true)
                .map_err(|error| Failure::retry(error.to_string()))?;
        }
        // Only names learned since the last write; the rest of the cache is
        // already in the journal.
        let pending = resolver.pending();
        if !pending.is_empty() {
            for (aumid, name) in &pending {
                let _ = journal.remember_app_name(aumid, name, SOURCE);
            }
            resolver.forget_pending();
        }
        for (key, hash) in fresh {
            self.seen.insert(key, hash);
        }
        // The cache is bounded: first the notifications that left the center,
        // and if that is not enough, everything that was not seen right now.
        if self.seen.len() > SEEN_LIMIT {
            self.seen.retain(|key, _| current.contains(key));
        }
        self.fallback_used += fallback_used;
        Ok(Report {
            available: true,
            state: self.state_text(),
            visible,
            emitted: records.len(),
            app_info_missing: self.app_info_missing,
            fallback_used: self.fallback_used,
            error: None,
        })
    }
}

/// Ask the user for access (`RequestAccessAsync`) and return the answer.
fn request_access(listener: &Com) -> Result<i32, String> {
    let mut operation: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        (listener.vtbl::<ListenerVtbl>().request_access_async)(listener.0, &mut operation)
    };
    if hr < 0 {
        return Err(format!("RequestAccessAsync не принят (0x{:08x})", hr as u32));
    }
    let operation = unsafe { Com::from_raw(operation) }
        .ok_or_else(|| "RequestAccessAsync вернул пустой указатель".to_string())?;
    // `IAsyncOperation<UserNotificationListenerAccessStatus>`: the result is a
    // value, so `GetResults` writes the enumeration itself.
    await_value(operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare two identifiers (the `GUID` type of `win32` has no equality).
    fn same_guid(left: &GUID, right: &GUID) -> bool {
        left.data1 == right.data1
            && left.data2 == right.data2
            && left.data3 == right.data3
            && left.data4 == right.data4
    }

    fn observed() -> Observed {
        Observed {
            id: 18127,
            created_ticks: Some(133_500_000_000_000_000),
            expires_ticks: Some(133_500_600_000_000_000),
            aumid: Some("Vendor.Chat_123!App".to_string()),
            app_name: Some("Vendor Chat".to_string()),
            app_info_error: None,
            template: Some("ToastGeneric".to_string()),
            language: Some("ru-RU".to_string()),
            texts: vec![
                "Алиса".to_string(),
                "Привет".to_string(),
                "Третья строка".to_string(),
            ],
        }
    }

    #[test]
    fn a_notification_becomes_the_record_the_journal_expects() {
        let resolver = NameResolver::new();
        let record = build_record(&observed(), None, &resolver);
        assert_eq!(record.source, "uns");
        assert_eq!(record.notification_type, "toast");
        assert_eq!(record.windows_id, Some(18127));
        assert_eq!(record.aumid.as_deref(), Some("Vendor.Chat_123!App"));
        assert_eq!(record.app_name.as_deref(), Some("Vendor Chat"));
        assert_eq!(record.title.as_deref(), Some("Алиса"));
        assert_eq!(record.body.as_deref(), Some("Привет"));
        assert_eq!(record.extra_text, vec!["Третья строка".to_string()]);
        assert_eq!(
            record.created_utc.as_deref(),
            time::filetime_to_iso(133_500_000_000_000_000).as_deref()
        );
        assert_eq!(
            record.expires_utc.as_deref(),
            time::filetime_to_iso(133_500_600_000_000_000).as_deref()
        );
        assert_eq!(record.extra["binding_template"], json!("ToastGeneric"));
        assert_eq!(record.extra["binding_language"], json!("ru-RU"));
        assert_eq!(record.extra["app_info_error"], json!(null));
        assert_eq!(record.raw["parser"], json!("uns"));
        assert_eq!(record.raw["notification_id"], json!(18127));
        // The identity is the same one `wpndb` computes, which is what makes the
        // two observations of one notification merge into a single row.
        assert_eq!(record.dedup_key(), "Vendor.Chat_123!App|toast|18127");
    }

    #[test]
    fn a_missing_application_comes_from_the_notification_database() {
        let resolver = NameResolver::new();
        resolver.remember("Vendor.Chat_123!App", "Осведомлённый клиент");
        let mut item = observed();
        // This is what Windows 11 does: `AppInfo` answers `E_NOTIMPL`.
        item.aumid = None;
        item.app_name = None;
        item.app_info_error = Some(0x8000_4001);
        let record = build_record(&item, Some("Vendor.Chat_123!App"), &resolver);
        assert_eq!(record.aumid.as_deref(), Some("Vendor.Chat_123!App"));
        assert_eq!(record.app_name.as_deref(), Some("Осведомлённый клиент"));
        assert_eq!(record.dedup_key(), "Vendor.Chat_123!App|toast|18127");
        assert_eq!(record.extra["app_info_error"], json!("0x80004001"));
        assert_eq!(record.raw["app_info_error"], json!("0x80004001"));
    }

    #[test]
    fn without_an_application_the_record_is_still_readable() {
        let resolver = NameResolver::new();
        let mut item = observed();
        item.aumid = None;
        item.app_name = None;
        let record = build_record(&item, None, &resolver);
        assert_eq!(record.aumid, None);
        assert_eq!(record.dedup_key(), "?|toast|18127");
        assert_eq!(record.raw["app_user_model_id"], json!(null));
    }

    #[test]
    fn the_text_lines_keep_their_order() {
        let resolver = NameResolver::new();
        let mut item = observed();
        item.texts = vec!["Только заголовок".to_string()];
        let record = build_record(&item, None, &resolver);
        assert_eq!(record.title.as_deref(), Some("Только заголовок"));
        assert_eq!(record.body, None);
        assert!(record.extra_text.is_empty());

        item.texts = Vec::new();
        let record = build_record(&item, None, &resolver);
        assert_eq!(record.title, None);
        assert_eq!(record.body, None);
    }

    #[test]
    fn the_access_answer_is_explained() {
        assert_eq!(access_status_text(ACCESS_ALLOWED), "доступ разрешён");
        assert_eq!(
            access_status_text(ACCESS_DENIED),
            "доступ запрещён пользователем"
        );
        assert_eq!(access_status_text(0), "доступ ещё не определён");
        assert_eq!(access_status_text(7), "неизвестный ответ системы");
    }

    #[test]
    fn a_report_says_what_the_source_does() {
        let mut report = Report::disabled("Windows не разрешила доступ");
        assert!(!report.available);
        assert!(report.describe().contains("не читается"));
        assert!(report.describe().contains("Windows не разрешила доступ"));

        report = Report {
            available: true,
            state: "доступ разрешён".to_string(),
            visible: 20,
            emitted: 3,
            app_info_missing: true,
            fallback_used: 3,
            error: None,
        };
        let line = report.describe();
        assert!(line.contains("доступ разрешён"), "{line}");
        assert!(line.contains("видно 20"), "{line}");
        assert!(line.contains("имя программы из базы Windows"), "{line}");

        report.error = Some("список недоступен".to_string());
        assert!(report
            .describe()
            .contains("ошибка чтения: список недоступен"));
    }

    #[test]
    fn a_report_survives_the_status_file() {
        let report = Report {
            available: true,
            state: "доступ разрешён".to_string(),
            visible: 4,
            emitted: 2,
            app_info_missing: false,
            fallback_used: 1,
            error: None,
        };
        assert_eq!(Report::from_json(&report.to_json()), report);
        // A file without the key (written by an older version) is not an error.
        assert_eq!(Report::from_json(&json!({})), Report::disabled(""));
    }

    #[test]
    fn the_two_sources_merge_into_one_journal_row() {
        // This is the whole point of the fallback: a `wpndb` observation and an
        // `uns` observation of one notification must become a single row whose
        // `sources` lists both collectors.
        let directory = std::env::temp_dir().join("wnl-uns-merge");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch directory");
        let journal = Journal::open(&directory.join("history.db")).expect("journal");

        let mut from_database = build_record(&observed(), None, &NameResolver::new());
        from_database.source = wpndb::SOURCE.to_string();
        from_database.title = Some("Алиса".to_string());
        let first = journal.record_many(&[from_database], 0).expect("first write");
        assert_eq!(first.total(), 1);

        // The same notification seen through the API, without `AppInfo`.
        let mut item = observed();
        item.aumid = None;
        item.app_name = None;
        item.app_info_error = Some(0x8000_4001);
        let from_api = build_record(&item, Some("Vendor.Chat_123!App"), &NameResolver::new());
        let second = journal.record_many(&[from_api], 0).expect("second write");
        assert_eq!(second.merged, 1, "запись должна слиться с наблюдением wpndb");

        let sources = journal.sources().expect("sources");
        assert_eq!(
            sources,
            vec![("uns+wpndb".to_string(), 1)],
            "одно уведомление — одна строка с двумя источниками"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    // ------------------------------------------------------------- live checks
    // The three tests below read the real notification center.  They never fail
    // because of the machine: when the API, the permission or the database is
    // missing, they say so and return - the source itself behaves the same way.

    #[test]
    fn the_state_of_the_source_can_be_read() {
        match probe() {
            Ok(state) => println!("WinRT: {state}"),
            Err(reason) => println!("пропуск: {reason}"),
        }
    }

    #[test]
    fn the_objects_match_the_declared_interfaces() {
        let mut session = Session::default();
        if let Err(reason) = session.ensure() {
            println!("пропуск: {reason}");
            return;
        }
        let listener = session
            .listener
            .as_ref()
            .expect("слушатель создан")
            .clone_ref();
        let iids = unsafe { listener.iids() };
        assert!(
            iids.iter()
                .any(|iid| same_guid(iid, &IID_USER_NOTIFICATION_LISTENER)),
            "объект не реализует интерфейс, объявленный в модуле"
        );
        let Ok(notifications) = read_notifications(&listener) else {
            println!("пропуск: список уведомлений недоступен");
            return;
        };
        println!("в центре уведомлений: {}", notifications.len());
        for notification in notifications.iter().take(3) {
            let iids = unsafe { notification.iids() };
            assert!(
                iids.iter()
                    .any(|iid| same_guid(iid, &IID_USER_NOTIFICATION)),
                "элемент списка не реализует IUserNotification"
            );
            let observed = observe(notification).expect("уведомление читается");
            assert!(observed.id > 0, "идентификатор уведомления");
            // Every identifier declared in this module is checked against the
            // object that carries it: a wrong constant would break the QI (and a
            // wrong slot would be caught by the values read below).
            let toast = unsafe {
                object_property(
                    notification.0,
                    notification.vtbl::<UserNotificationVtbl>().get_notification,
                )
            }
            .expect("уведомление содержит toast");
            assert!(
                unsafe { toast.query(&IID_NOTIFICATION) }.is_ok(),
                "тост не реализует INotification"
            );
            let visual = unsafe { object_property(toast.0, toast.vtbl::<NotificationVtbl>().get_visual) }
                .expect("у тоста есть visual");
            assert!(
                unsafe { visual.query(&IID_NOTIFICATION_VISUAL) }.is_ok(),
                "visual не реализует INotificationVisual"
            );
            let name = Hstring::new(TOAST_GENERIC).expect("строка WinRT");
            let mut pointer: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                (visual.vtbl::<NotificationVisualVtbl>().get_binding)(visual.0, name.0, &mut pointer)
            };
            assert!(hr >= 0, "GetBinding(ToastGeneric): 0x{:08x}", hr as u32);
            let binding = unsafe { Com::from_raw(pointer) }.expect("привязка ToastGeneric");
            assert!(
                unsafe { binding.query(&IID_NOTIFICATION_BINDING) }.is_ok(),
                "привязка не реализует INotificationBinding"
            );
            let texts = read_texts(&binding);
            assert_eq!(texts, observed.texts, "тексты читаются одинаково");
            // The text element closes the chain of interfaces declared here.
            if let Some(elements) = unsafe {
                object_property(
                    binding.0,
                    binding.vtbl::<NotificationBindingVtbl>().get_text_elements,
                )
            } {
                let mut pointer: *mut c_void = std::ptr::null_mut();
                let hr = unsafe {
                    (elements.vtbl::<VectorViewVtbl>().get_at)(elements.0, 0, &mut pointer)
                };
                if hr >= 0 {
                    if let Some(element) = unsafe { Com::from_raw(pointer) } {
                        assert!(
                            unsafe { element.query(&IID_ADAPTIVE_NOTIFICATION_TEXT) }.is_ok(),
                            "текстовый элемент не реализует IAdaptiveNotificationText"
                        );
                    }
                }
            }
            if observed.app_info_error.is_none() {
                // Windows 10 reports the application; the two interfaces of that
                // object are declared here as well.
                let info = unsafe {
                    object_property(
                        notification.0,
                        notification.vtbl::<UserNotificationVtbl>().get_app_info,
                    )
                };
                if let Some(info) = info {
                    assert!(unsafe { info.query(&IID_APP_INFO) }.is_ok(), "AppInfo");
                    if let Some(display) =
                        unsafe { object_property(info.0, info.vtbl::<AppInfoVtbl>().get_display_info) }
                    {
                        assert!(
                            unsafe { display.query(&IID_APP_DISPLAY_INFO) }.is_ok(),
                            "AppDisplayInfo"
                        );
                    }
                }
            }
            // A `FILETIME` after the year 2000: the value proves that the property
            // sits in the slot this module declares.
            if let Some(ticks) = observed.created_ticks {
                assert!(
                    ticks > 125_911_584_000_000_000,
                    "время создания выглядит неверно: {ticks}"
                );
            }
            if let Some(template) = &observed.template {
                assert_eq!(template, TOAST_GENERIC, "шаблон привязки");
            }
            println!(
                "уведомление {}: строк текста {}, срок задан: {}, шаблон {:?}",
                observed.id,
                observed.texts.len(),
                observed.expires_ticks.is_some(),
                observed.template
            );
        }
    }

    #[test]
    fn the_database_knows_the_application_of_a_visible_notification() {
        // The fallback for `AppInfo` only works because both sources share the
        // identifier; this checks that assumption on the running system.
        let Ok(database) = wpndb::NotificationDatabase::open(&wpndb::database_path()) else {
            println!("пропуск: база уведомлений недоступна");
            return;
        };
        let mut session = Session::default();
        if let Err(reason) = session.ensure() {
            println!("пропуск: {reason}");
            return;
        }
        let listener = session.listener.as_ref().expect("слушатель").clone_ref();
        let Ok(notifications) = read_notifications(&listener) else {
            println!("пропуск: список уведомлений недоступен");
            return;
        };
        let mut checked = 0;
        for notification in &notifications {
            let Ok(observed) = observe(notification) else {
                continue;
            };
            if observed.aumid.is_some() {
                continue; // the platform named the application itself
            }
            let from_database = database.aumid_for_id(i64::from(observed.id)).unwrap_or(None);
            println!(
                "уведомление {}: приложение из базы {:?}",
                observed.id, from_database
            );
            checked += 1;
            if checked >= 3 {
                break;
            }
        }
        println!("проверено уведомлений без имени программы: {checked}");
    }
}
