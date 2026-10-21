use std::{fmt::Write, os::windows::ffi::OsStringExt};

#[derive(Debug, Default, Clone)]
pub(crate) struct ProcessInfo {
    pub(crate) pid: u32,
    pub(crate) ppid: u32,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) subsystem: std::rc::Rc<str>, // here, because need to know where process comes from when killing, and do not want multiple copies
    pub(crate) cpu_usage_percent: f64,
    pub(crate) memory_usage_percent: f64,
    pub(crate) kernel_time: u64,
    pub(crate) user_time: u64,
    pub(crate) start_time_unix: Option<i64>,
}

pub struct OwnedHandle(windows::Win32::Foundation::HANDLE);

impl OwnedHandle {
    pub fn new(handle: windows::Win32::Foundation::HANDLE) -> Self {
        Self(handle)
    }

    pub fn raw(&self) -> windows::Win32::Foundation::HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn process_times(handle: windows::Win32::Foundation::HANDLE) -> Option<(u64, u64, Option<i64>)> {
    let mut creation_time = windows::Win32::Foundation::FILETIME::default();
    let mut exit_time = windows::Win32::Foundation::FILETIME::default();
    let mut kernel_time = windows::Win32::Foundation::FILETIME::default();
    let mut user_time = windows::Win32::Foundation::FILETIME::default();

    let ok = unsafe {
        windows::Win32::System::Threading::GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    }
    .is_ok();

    let output = if ok {
        Some((
            filetime_to_u64(kernel_time),
            filetime_to_u64(user_time),
            filetime_to_unix_time(creation_time),
        ))
    } else {
        None
    };

    output
}

fn executable_path(
    handle: windows::Win32::Foundation::HANDLE,
    buffer: &mut Vec<u16>,
) -> Option<String> {
    if buffer.is_empty() {
        buffer.resize(260, 0);
    }

    loop {
        let mut len = u32::try_from(buffer.len()).ok()?;

        match unsafe {
            windows::Win32::System::Threading::QueryFullProcessImageNameW(
                handle,
                windows::Win32::System::Threading::PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
        } {
            Ok(()) => {
                buffer.truncate(len as usize);
                return Some(String::from_utf16_lossy(buffer));
            }
            Err(error)
                if error.code()
                    == windows::core::HRESULT::from_win32(
                        windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER.0,
                    ) =>
            {
                let new_len = buffer.len().checked_mul(2)?;
                buffer.resize(new_len, 0);
            }
            Err(_) => return None,
        }
    }
}
fn process_memory_percent(
    handle: windows::Win32::Foundation::HANDLE,
    total_physical_memory: u64,
) -> f64 {
    if total_physical_memory == 0 {
        return -1.0;
    }

    let mut counters = windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS::default();
    counters.cb = std::mem::size_of_val(&counters) as u32;

    let ok = unsafe {
        windows::Win32::System::ProcessStatus::GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of_val(&counters) as u32,
        )
    }
    .is_ok();

    let result = if ok {
        counters.WorkingSetSize as f64 / total_physical_memory as f64 * 100.0
    } else {
        -1.0
    };

    result
}

fn total_physical_memory() -> Result<u64> {
    let mut info = windows::Win32::System::ProcessStatus::PERFORMANCE_INFORMATION::default();
    info.cb = std::mem::size_of_val(&info) as u32;
    unsafe {
        windows::Win32::System::ProcessStatus::GetPerformanceInfo(
            &mut info,
            std::mem::size_of_val(&info) as u32,
        )?;
    }
    Ok(info.PhysicalTotal as u64 * info.PageSize as u64)
}

fn logical_cpu_count() -> u32 {
    let mut info = windows::Win32::System::SystemInformation::SYSTEM_INFO::default();
    unsafe {
        windows::Win32::System::SystemInformation::GetSystemInfo(&mut info);
    }
    info.dwNumberOfProcessors.max(1)
}

fn filetime_to_u64(value: windows::Win32::Foundation::FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

fn filetime_to_unix_time(value: windows::Win32::Foundation::FILETIME) -> Option<i64> {
    // FILETIME starts at 1601-01-01.
    // Unix time starts at 1970-01-01.
    const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 11644473600 * 10_000_000;

    let filetime = filetime_to_u64(value);

    if filetime < WINDOWS_TO_UNIX_EPOCH_100NS {
        return None;
    }

    Some(((filetime - WINDOWS_TO_UNIX_EPOCH_100NS) / 10_000_000) as i64)
}

// Parses ps cputime forms such as "12:34", "1:12:34", or "2-01:12:34"
// and converts to 100-nanosecond units.
fn parse_ps_cputime_100ns(s: &str) -> Option<u64> {
    let (days, clock) = match s.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, s),
    };

    let parts: Vec<u64> = clock
        .split(':')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;

    let seconds = match parts.as_slice() {
        [minutes, seconds] => days * 86_400 + minutes * 60 + seconds,
        [hours, minutes, seconds] => days * 86_400 + hours * 3_600 + minutes * 60 + seconds,
        _ => return None,
    };

    Some(seconds.saturating_mul(10_000_000))
}

fn collect_process_samples(
) -> Result<std::collections::HashMap<u32, (ProcessInfo, Option<OwnedHandle>)>> {
    let snapshot = OwnedHandle(unsafe {
        windows::Win32::System::Diagnostics::ToolHelp::CreateToolhelp32Snapshot(
            windows::Win32::System::Diagnostics::ToolHelp::TH32CS_SNAPPROCESS,
            0,
        )?
    });

    let mut entry = windows::Win32::System::Diagnostics::ToolHelp::PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of_val(&entry) as u32;

    let mut processes = std::collections::HashMap::new();

    if unsafe {
        windows::Win32::System::Diagnostics::ToolHelp::Process32FirstW(snapshot.raw(), &mut entry)
    }
    .is_ok()
    {
        loop {
            let pid = entry.th32ProcessID;
            let ppid = entry.th32ParentProcessID;

            let handle = unsafe {
                windows::Win32::System::Threading::OpenProcess(
                    windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION
                        | windows::Win32::System::Threading::PROCESS_QUERY_INFORMATION,
                    false,
                    pid,
                )
            }
            .ok()
            .map(OwnedHandle);

            // either previous query failed or !fullpath
            let name = {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());

                std::ffi::OsString::from_wide(&entry.szExeFile[..end])
                    .to_string_lossy()
                    .into_owned()
            };

            let (kernel_time, user_time, start_time_unix) = handle
                .as_ref()
                .and_then(|h| process_times(h.raw()))
                .unwrap_or_default();

            processes.insert(
                pid,
                (
                    ProcessInfo {
                        pid,
                        ppid,
                        name,
                        kernel_time,
                        user_time,
                        start_time_unix,
                        ..Default::default()
                    },
                    handle,
                ),
            );

            if unsafe {
                windows::Win32::System::Diagnostics::ToolHelp::Process32NextW(
                    snapshot.raw(),
                    &mut entry,
                )
            }
            .is_err()
            {
                break;
            }
        }
    }

    Ok(processes)
}

pub(crate) fn get_processes(
    fullpath: bool,
    previous: &[ProcessInfo],
    elapsed: std::time::Duration,
) -> Result<Vec<ProcessInfo>> {
    let total_physical_memory = total_physical_memory()?;
    let logical_cpus = logical_cpu_count();

    let first: std::collections::HashMap<_, _> = previous
        .iter()
        .filter(|p| p.subsystem.as_ref() == "Windows")
        .map(|p| (p.pid, p))
        .collect();
    let second = collect_process_samples()?;
    let elapsed_100ns = elapsed.as_secs_f64() * 10_000_000.0;

    let mut processes = Vec::with_capacity(second.len());

    let mut path_buffer = Vec::new();
    for process in second {
        let cpu_usage_percent = match first.get(&process.1 .0.pid) {
            Some(previous)
                if previous.start_time_unix.is_some()
                    && previous.start_time_unix == process.1 .0.start_time_unix
                    && elapsed_100ns > 0.0 =>
            {
                let process_delta = process
                    .1
                     .0
                    .kernel_time
                    .saturating_sub(previous.kernel_time)
                    + process.1 .0.user_time.saturating_sub(previous.user_time);

                // Process time is measured in 100 ns units.
                // Divide by elapsed wall-clock time and logical CPU count.
                ((process_delta as f64 / elapsed_100ns as f64) * 100.0 / logical_cpus as f64)
                    .clamp(0.0, 100.0)
            }
            _ => -1.0,
        };

        let memory_usage_percent = process
            .1
             .1
            .as_ref()
            .map(|h| process_memory_percent(h.raw(), total_physical_memory))
            .unwrap_or(-1.0);

        let fullname = if fullpath {
            process
                .1
                 .1
                .as_ref()
                .and_then(|h| executable_path(h.raw(), &mut path_buffer))
        } else {
            None
        };

        processes.push(ProcessInfo {
            pid: process.1 .0.pid,
            ppid: process.1 .0.ppid,
            name: fullname.unwrap_or_else(|| process.1 .0.name.clone()),
            prefix: String::new(),
            subsystem: std::rc::Rc::from("Windows"),
            cpu_usage_percent,
            memory_usage_percent,
            start_time_unix: process.1 .0.start_time_unix,
            kernel_time: process.1 .0.kernel_time,
            user_time: process.1 .0.user_time,
        });
    }

    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}

// NOTE: might not kill children
pub(crate) fn force_kill(pi: &ProcessInfo) -> std::io::Result<()> {
    let handle = OwnedHandle(
        unsafe {
            windows::Win32::System::Threading::OpenProcess(
                windows::Win32::System::Threading::PROCESS_TERMINATE,
                false,
                pi.pid,
            )
        }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 as i32))?,
    );

    let result = unsafe { windows::Win32::System::Threading::TerminateProcess(handle.raw(), 1) };

    result.map_err(|e| std::io::Error::from_raw_os_error(e.code().0 as i32))?;
    Ok(())
}

#[derive(Default)]
struct CpuSampler {
    query: Option<windows::Win32::System::Performance::PDH_HQUERY>,
    counters: Vec<windows::Win32::System::Performance::PDH_HCOUNTER>,
    primed: bool,
}

impl CpuSampler {
    fn initialize(&mut self) -> Result<()> {
        if self.query.is_some() {
            return Ok(());
        }

        let mut query = windows::Win32::System::Performance::PDH_HQUERY::default();
        let status = unsafe {
            windows::Win32::System::Performance::PdhOpenQueryW(
                windows::core::PCWSTR::null(),
                0,
                &mut query,
            )
        };
        if status != 0 {
            return Err(std::io::Error::other(format!("PDH error 0x{status:08X}")).into());
        }
        self.query = Some(query);

        let group_count =
            unsafe { windows::Win32::System::Threading::GetActiveProcessorGroupCount() };
        let mut buffer = String::new();
        for group in 0..group_count {
            let processor_count =
                unsafe { windows::Win32::System::Threading::GetActiveProcessorCount(group) };
            for processor in 0..processor_count {
                buffer.clear();
                write!(
                    &mut buffer,
                    "\\Processor Information({group},{processor})\\% Processor Time"
                )?;
                let path: Vec<u16> = buffer.encode_utf16().chain(Some(0)).collect();
                let mut counter = windows::Win32::System::Performance::PDH_HCOUNTER::default();
                let status = unsafe {
                    windows::Win32::System::Performance::PdhAddEnglishCounterW(
                        query,
                        windows::core::PCWSTR(path.as_ptr()),
                        0,
                        &mut counter,
                    )
                };
                if status != 0 {
                    return Err(std::io::Error::other(format!("PDH error 0x{status:08X}")).into());
                }
                self.counters.push(counter);
            }
        }

        Ok(())
    }

    fn sample(&mut self, usage_percent: &mut Vec<f64>) -> Result<()> {
        use windows::Win32::System::Performance::{
            PdhCollectQueryData, PdhGetFormattedCounterValue, PDH_CALC_NEGATIVE_DENOMINATOR,
            PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
        };

        self.initialize()?;
        let query = self.query.expect("initialized PDH query");
        let status = unsafe { PdhCollectQueryData(query) };
        if status != 0 {
            return Err(std::io::Error::other(format!("PDH error 0x{status:08X}")).into());
        }
        usage_percent.clear();
        usage_percent.resize(self.counters.len(), 0.0);
        if !self.primed {
            self.primed = true;
            return Ok(());
        }

        for (usage, counter) in usage_percent.iter_mut().zip(&self.counters) {
            let mut value = PDH_FMT_COUNTERVALUE::default();
            let status =
                unsafe { PdhGetFormattedCounterValue(*counter, PDH_FMT_DOUBLE, None, &mut value) };
            if status == windows::Win32::System::Performance::PDH_INVALID_DATA
                || status == PDH_CALC_NEGATIVE_DENOMINATOR
            {
                continue;
            }
            if status != 0 {
                return Err(std::io::Error::other(format!("PDH error 0x{status:08X}")).into());
            }

            if value.CStatus == PDH_CSTATUS_VALID_DATA || value.CStatus == PDH_CSTATUS_NEW_DATA {
                *usage = unsafe { value.Anonymous.doubleValue.clamp(0.0, 100.0) };
            }
        }

        Ok(())
    }
}

impl Drop for CpuSampler {
    fn drop(&mut self) {
        if let Some(query) = self.query.take() {
            unsafe {
                windows::Win32::System::Performance::PdhCloseQuery(query);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct SystemMetrics {
    pub(crate) cpu_sampler: CpuSampler,
    pub(crate) cpu_usage_percent: Vec<f64>,
    pub(crate) memory_usage_percent: f64,
    pub(crate) uptime_seconds: u64,
}

impl SystemMetrics {
    pub(crate) fn sample(&mut self) -> Result<()> {
        self.cpu_sampler.sample(&mut self.cpu_usage_percent)?;

        let mut memory_info =
            windows::Win32::System::ProcessStatus::PERFORMANCE_INFORMATION::default();
        memory_info.cb = std::mem::size_of_val(&memory_info) as u32;
        unsafe {
            windows::Win32::System::ProcessStatus::GetPerformanceInfo(
                &mut memory_info,
                std::mem::size_of_val(&memory_info) as u32,
            )?;
        }
        let total_pages = memory_info.PhysicalTotal as u64;
        let available_pages = memory_info.PhysicalAvailable as u64;
        let memory_usage_percent = if total_pages == 0 {
            0.0
        } else {
            (total_pages.saturating_sub(available_pages) as f64 / total_pages as f64 * 100.0)
                .clamp(0.0, 100.0)
        };

        let uptime_seconds =
            unsafe { windows::Win32::System::SystemInformation::GetTickCount64() / 1_000 };

        self.memory_usage_percent = memory_usage_percent;
        self.uptime_seconds = uptime_seconds;
        Ok(())
    }
}

/// Returns one formatted line per process
pub(crate) fn process_tree_lines(processes: &[&ProcessInfo]) -> Vec<ProcessInfo> {
    use std::collections::{BTreeMap, HashSet};

    let mut children: BTreeMap<u32, Vec<&ProcessInfo>> = BTreeMap::new();
    let mut known_pids = HashSet::new();

    for process in processes {
        known_pids.insert(process.pid);
        children.entry(process.ppid).or_default().push(process);
    }

    // A process is a root if its parent is not in the supplied process list.
    let roots: Vec<&ProcessInfo> = processes
        .iter()
        .copied()
        .filter(|process| process.ppid == 0 || !known_pids.contains(&process.ppid))
        .collect();

    let mut lines = Vec::new();
    let mut visited = HashSet::new();

    for (index, root) in roots.iter().enumerate() {
        let is_last_root = index + 1 == roots.len();

        render_process(
            root,
            "",
            true,
            is_last_root,
            &children,
            &mut visited,
            &mut lines,
        );
    }
    lines
}

fn render_process<'a>(
    process: &'a ProcessInfo,
    prefix: &str,
    is_root: bool,
    is_last: bool,
    children: &std::collections::BTreeMap<u32, Vec<&'a ProcessInfo>>,
    visited: &mut std::collections::HashSet<u32>,
    lines: &mut Vec<ProcessInfo>,
) {
    // Prevent infinite recursion if the process list contains a cycle.
    if !visited.insert(process.pid) {
        return;
    }

    let branch = if is_root {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };

    lines.push(ProcessInfo {
        pid: process.pid,
        ppid: process.ppid,
        name: process.name.clone(),
        prefix: format!("{prefix}{branch}"),
        subsystem: process.subsystem.clone(),
        cpu_usage_percent: process.cpu_usage_percent,
        memory_usage_percent: process.memory_usage_percent,
        start_time_unix: process.start_time_unix,
        kernel_time: process.kernel_time,
        user_time: process.user_time,
    });

    if let Some(process_children) = children.get(&process.pid) {
        for (index, child) in process_children.iter().enumerate() {
            let child_is_last = index + 1 == process_children.len();

            let child_prefix = if is_root {
                String::new()
            } else if is_last {
                format!("{prefix}   ")
            } else {
                format!("{prefix}│  ")
            };

            render_process(
                child,
                &child_prefix,
                false,
                child_is_last,
                children,
                visited,
                lines,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::common::process;

    use super::*;

    #[test]
    fn wininit_is_listed() {
        let processes = get_processes(false, &[], std::time::Duration::ZERO).unwrap();
        let wininit = processes
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("wininit.exe"))
            .expect("wininit should always be there");

        assert_eq!(wininit.subsystem.as_ref(), "Windows");
        assert_eq!(wininit.cpu_usage_percent, -1.0);

        let process_refs: Vec<&ProcessInfo> = processes.iter().collect();
        assert!(process_tree_lines(&process_refs)
            .iter()
            .any(|p| p.pid == wininit.pid));
    }

    #[test]
    fn process_tree_is_rendered_correctly() {
        let mut processes = vec![
            ProcessInfo {
                pid: 200,
                ppid: 100,
                name: "explorer.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 100,
                ppid: 0,
                name: "init.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 300,
                ppid: 100,
                name: "services.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 400,
                ppid: 200,
                name: "chrome.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 500,
                ppid: 200,
                name: "notepad2.exe".to_string(),
                ..Default::default()
            },
            ProcessInfo {
                pid: 600,
                ppid: 500,
                name: "notepad.exe".to_string(),
                ..Default::default()
            },
        ];

        processes.sort_by_key(|process| process.pid);
        let process_refs: Vec<&ProcessInfo> = processes.iter().collect();
        let actual: Vec<String> = process_tree_lines(&process_refs)
            .into_iter()
            .map(|p| format!("{} | {}{}", p.pid, p.prefix, p.name))
            .collect();

        let expected = vec![
            "100 | init.exe",
            "200 | ├─ explorer.exe",
            "400 | │  ├─ chrome.exe",
            "500 | │  └─ notepad2.exe",
            "600 | │     └─ notepad.exe",
            "300 | └─ services.exe",
        ];

        let expected: Vec<String> = expected.into_iter().map(String::from).collect();

        assert_eq!(actual, expected);
    }
}
