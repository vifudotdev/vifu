use std::time::{Duration, Instant};

use super::model::SystemMetrics;

#[derive(Clone, Copy, Debug)]
struct CpuPoint {
    at: Instant,
    process_time: Duration,
}

#[derive(Default)]
pub(crate) struct SystemSampler {
    previous_cpu: Option<CpuPoint>,
}

impl SystemSampler {
    pub(crate) fn sample(&mut self) -> SystemMetrics {
        let current_cpu = process_cpu_point();
        let cpu_percent = self
            .previous_cpu
            .zip(current_cpu)
            .and_then(|(previous, current)| {
                let wall = current.at.duration_since(previous.at).as_secs_f64();
                (wall > 0.0).then(|| {
                    let process = current
                        .process_time
                        .saturating_sub(previous.process_time)
                        .as_secs_f64();
                    (process / wall * 100.0).max(0.0)
                })
            });
        self.previous_cpu = current_cpu;

        SystemMetrics {
            cpu_percent,
            rss_bytes: current_rss_bytes(),
            total_memory_bytes: total_memory_bytes(),
        }
    }
}

#[cfg(target_os = "linux")]
fn process_cpu_point() -> Option<CpuPoint> {
    Some(CpuPoint {
        at: Instant::now(),
        process_time: current_process_cpu_time()?,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn current_process_cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let ticks = parse_linux_process_ticks(&stat)?;
    // SAFETY: sysconf reads a process-global constant and does not dereference pointers.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        ticks as f64 / ticks_per_second as f64,
    ))
}

#[cfg(target_os = "linux")]
fn parse_linux_process_ticks(stat: &str) -> Option<u64> {
    let command_end = stat.rfind(')')?;
    let fields = stat
        .get(command_end.saturating_add(2)..)?
        .split_whitespace();
    let values = fields.collect::<Vec<_>>();
    let user = values.get(11)?.parse::<u64>().ok()?;
    let system = values.get(12)?.parse::<u64>().ok()?;
    Some(user.saturating_add(system))
}

#[cfg(target_os = "linux")]
pub(crate) fn current_rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    // SAFETY: sysconf reads a process-global constant and does not dereference pointers.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0).then(|| resident_pages.saturating_mul(page_size as u64))
}

#[cfg(target_os = "linux")]
fn total_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_linux_total_memory(&meminfo)
}

#[cfg(target_os = "linux")]
fn parse_linux_total_memory(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib.saturating_mul(1024))
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct RusageInfoV2 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    pkg_idle_wakeups: u64,
    interrupt_wakeups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    physical_footprint: u64,
    process_start_abstime: u64,
    process_exit_abstime: u64,
    child_user_time: u64,
    child_system_time: u64,
    child_pkg_idle_wakeups: u64,
    child_interrupt_wakeups: u64,
    child_pageins: u64,
    child_elapsed_abstime: u64,
}

#[cfg(target_os = "macos")]
impl Default for RusageInfoV2 {
    fn default() -> Self {
        Self {
            uuid: [0; 16],
            user_time: 0,
            system_time: 0,
            pkg_idle_wakeups: 0,
            interrupt_wakeups: 0,
            pageins: 0,
            wired_size: 0,
            resident_size: 0,
            physical_footprint: 0,
            process_start_abstime: 0,
            process_exit_abstime: 0,
            child_user_time: 0,
            child_system_time: 0,
            child_pkg_idle_wakeups: 0,
            child_interrupt_wakeups: 0,
            child_pageins: 0,
            child_elapsed_abstime: 0,
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn proc_pid_rusage(
        pid: libc::c_int,
        flavor: libc::c_int,
        buffer: *mut libc::c_void,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
fn macos_rusage() -> Option<RusageInfoV2> {
    const RUSAGE_INFO_V2: libc::c_int = 2;
    let mut usage = RusageInfoV2::default();
    // SAFETY: `usage` is a correctly sized writable rusage_info_v2 buffer for this process.
    let result = unsafe {
        proc_pid_rusage(
            libc::getpid(),
            RUSAGE_INFO_V2,
            std::ptr::addr_of_mut!(usage).cast(),
        )
    };
    (result == 0).then_some(usage)
}

#[cfg(target_os = "macos")]
fn process_cpu_point() -> Option<CpuPoint> {
    Some(CpuPoint {
        at: Instant::now(),
        process_time: current_process_cpu_time()?,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn current_process_cpu_time() -> Option<Duration> {
    let usage = macos_rusage()?;
    Some(Duration::from_nanos(
        usage.user_time.saturating_add(usage.system_time),
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn current_rss_bytes() -> Option<u64> {
    Some(macos_rusage()?.resident_size)
}

#[cfg(target_os = "macos")]
fn total_memory_bytes() -> Option<u64> {
    let mut value = 0_u64;
    let mut size = std::mem::size_of::<u64>();
    let name = std::ffi::CString::new("hw.memsize").ok()?;
    // SAFETY: `value` and `size` are valid writable buffers and the sysctl name is NUL-terminated.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::addr_of_mut!(value).cast(),
            std::ptr::addr_of_mut!(size),
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then_some(value)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cpu_point() -> Option<CpuPoint> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn current_process_cpu_time() -> Option<Duration> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn current_rss_bytes() -> Option<u64> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn total_memory_bytes() -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{parse_linux_process_ticks, parse_linux_total_memory};

    #[test]
    fn process_stat_should_parse_user_and_system_ticks_after_parenthesized_name() {
        let stat = "123 (vifu worker) R 1 2 3 4 5 6 7 8 9 10 120 30 0 0";

        assert_eq!(parse_linux_process_ticks(stat), Some(150));
    }

    #[test]
    fn meminfo_should_parse_total_memory_in_bytes() {
        let meminfo = "MemTotal:       16384 kB\nMemFree: 1024 kB\n";

        assert_eq!(parse_linux_total_memory(meminfo), Some(16_777_216));
    }
}
