//! Opens the original launcher and waits until it and the game have closed.
//!
//! The game does not always run inside the launcher: newer versions are separate processes, and
//! the launcher closes when they start. This process joins a job object first, so the launcher and
//! every game inherit it, and everything is over once this process is the only one left in it.

use std::{io, mem, process::Command, ptr, thread, time::Duration};

use windows_sys::Win32::{
    Foundation::{BOOL, HANDLE, HWND, LPARAM},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
            QueryInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        },
        Threading::GetCurrentProcess,
    },
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
};

pub fn play(mut launcher: Command) -> io::Result<()> {
    // SAFETY: plain Win32 calls; the job handle stays open for the life of the process.
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() || unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    launcher.spawn()?;
    while active_processes(job)? > 1 {
        thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn active_processes(job: HANDLE) -> io::Result<u32> {
    // SAFETY: the buffer is a correctly sized JOBOBJECT_BASIC_ACCOUNTING_INFORMATION.
    unsafe {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = mem::zeroed();
        let ok = QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            mem::size_of_val(&info) as u32,
            ptr::null_mut(),
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses)
    }
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
