//! A progress dialog for the one wait that happens before the launcher can open: downloading
//! its Java the first time.

use std::{
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, PoisonError,
    },
    thread,
};

use windows_sys::{
    core::HRESULT,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{
                TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_CANCEL_BUTTON, TDE_CONTENT, TDF_CALLBACK_TIMER,
                TDF_SHOW_PROGRESS_BAR, TDM_CLICK_BUTTON, TDM_SET_PROGRESS_BAR_POS, TDM_UPDATE_ELEMENT_TEXT,
                TDN_TIMER,
            },
            WindowsAndMessaging::{SendMessageW, IDCANCEL},
        },
    },
};

/// What the work reports: a percentage and a line of text.
pub type Report<'a> = &'a (dyn Fn(u64, String) + Sync);

#[derive(Default)]
struct Shared {
    percent: AtomicU64,
    text: Mutex<String>,
    shown: Mutex<String>,
    finished: AtomicBool,
}

/// Runs `work` on another thread while a dialog shows its progress. Returns `None` if the player
/// pressed Cancel; the work is then left to end with the process.
pub fn progress<T: Send + 'static>(
    instruction: &str,
    first_text: &str,
    work: impl FnOnce(Report) -> T + Send + 'static,
) -> Option<T> {
    let shared = Arc::new(Shared { text: Mutex::new(first_text.to_string()), ..Shared::default() });
    let worker = thread::spawn({
        let shared = shared.clone();
        move || {
            let result = work(&|percent, text| {
                shared.percent.store(percent.min(100), Ordering::Relaxed);
                *shared.text.lock().unwrap_or_else(PoisonError::into_inner) = text;
            });
            shared.finished.store(true, Ordering::Release);
            result
        }
    });

    let title = wide("Minecraft Launcher Classic");
    let instruction = wide(instruction);
    let content = wide(first_text);
    // SAFETY: every pointer refers to a NUL-terminated buffer or to `shared`, all of which outlive
    // the modal TaskDialogIndirect call.
    let shown = unsafe {
        let mut config: TASKDIALOGCONFIG = std::mem::zeroed();
        config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
        config.dwFlags = TDF_SHOW_PROGRESS_BAR | TDF_CALLBACK_TIMER;
        config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
        config.pszWindowTitle = title.as_ptr();
        // The exe's icon, resource 1 (build.rs), also on the title bar.
        config.hInstance = GetModuleHandleW(ptr::null());
        config.Anonymous1 = TASKDIALOGCONFIG_0 { pszMainIcon: ptr::without_provenance(1) }; // MAKEINTRESOURCE(1)
        config.pszMainInstruction = instruction.as_ptr();
        config.pszContent = content.as_ptr();
        config.cxWidth = 250; // dialog units: room for the texts on one line each
        config.pfCallback = Some(callback);
        config.lpCallbackData = Arc::as_ptr(&shared) as isize;
        TaskDialogIndirect(&config, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) >= 0
    };
    if shown && !shared.finished.load(Ordering::Acquire) {
        return None;
    }
    worker.join().ok()
}

unsafe extern "system" fn callback(dialog: HWND, notification: i32, _: WPARAM, _: LPARAM, data: isize) -> HRESULT {
    if notification != TDN_TIMER {
        return 0;
    }
    // SAFETY: `data` is the `Shared` that `progress` keeps alive while the dialog is open.
    let shared = unsafe { &*(data as *const Shared) };
    unsafe {
        if shared.finished.load(Ordering::Acquire) {
            SendMessageW(dialog, TDM_CLICK_BUTTON as u32, IDCANCEL as WPARAM, 0);
            return 0;
        }
        SendMessageW(dialog, TDM_SET_PROGRESS_BAR_POS as u32, shared.percent.load(Ordering::Relaxed) as WPARAM, 0);
        let text = shared.text.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let mut shown = shared.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if *shown != text {
            let buffer = wide(&text);
            SendMessageW(dialog, TDM_UPDATE_ELEMENT_TEXT as u32, TDE_CONTENT as WPARAM, buffer.as_ptr() as LPARAM);
            *shown = text;
        }
    }
    0
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
