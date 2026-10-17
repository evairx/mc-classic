//! Opens the original launcher and waits until it closes. A game that runs apart from it (see
//! `bridge.rs`) is not waited for: it outlives the launcher and this program.

use std::{io, process::Command};

use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM},
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
};

pub fn play(mut launcher: Command) -> io::Result<()> {
    launcher.spawn()?.wait()?;
    Ok(())
}

/// Whether the process has a visible top-level window, such as the game's.
pub fn has_visible_window(pid: u32) -> bool {
    struct Search {
        pid: u32,
        found: bool,
    }
    unsafe extern "system" fn visit(window: HWND, search: LPARAM) -> BOOL {
        // SAFETY: `search` is the `Search` that EnumWindows was given below, alive for the call.
        let search = unsafe { &mut *(search as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(window, &mut owner) };
        if owner == search.pid && unsafe { IsWindowVisible(window) } != 0 {
            search.found = true;
            return 0; // stop
        }
        1
    }
    let mut search = Search { pid, found: false };
    // SAFETY: the callback only touches `search`, which outlives the call.
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    search.found
}
