use serde::Serialize;

pub const MIN: usize = 2;
pub const DEFAULT: usize = 4;
pub const MAX: usize = 16;
const GIB: u64 = 1024 * 1024 * 1024;

pub fn is_supported(lanes: usize) -> bool {
    (MIN..=MAX).contains(&lanes) && lanes % 2 == 0
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub logical_cores: Option<usize>,
    pub memory_bytes: Option<u64>,
    pub recommended_concurrency: usize,
}

impl HardwareProfile {
    pub fn detect() -> Self {
        let logical_cores = std::thread::available_parallelism().ok().map(usize::from);
        let memory_bytes = total_memory();
        Self {
            logical_cores,
            memory_bytes,
            recommended_concurrency: recommend(logical_cores, memory_bytes),
        }
    }
}

fn recommend(logical_cores: Option<usize>, memory_bytes: Option<u64>) -> usize {
    // Leave CPU headroom for the OS and previews, including on SMT processors.
    let cpu_limit = logical_cores.map_or(DEFAULT, |cores| cores / 2);
    // Reserve 4 GiB for the system and shared weights, plus a conservative
    // 1.5 GiB per lane for inference and rendering. This is a budget, not RSS.
    let memory_limit = memory_bytes.map_or(DEFAULT, |bytes| {
        (bytes.saturating_sub(4 * GIB) / (GIB + GIB / 2)) as usize
    });
    let budget = cpu_limit.min(memory_limit).clamp(MIN, MAX);
    budget - budget % 2
}

#[cfg(target_os = "macos")]
fn total_memory() -> Option<u64> {
    let mut bytes = 0u64;
    let mut length = std::mem::size_of::<u64>();
    // sysctl writes at most `length` bytes into the live u64 buffer.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && length == std::mem::size_of::<u64>() && bytes > 0).then_some(bytes)
}

#[cfg(windows)]
fn total_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // The struct has the required size and remains writable for the call.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0 && status.ullTotalPhys > 0).then_some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn total_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_respects_cpu_memory_and_bounds() {
        assert_eq!(recommend(Some(2), Some(4 * GIB)), 2);
        assert_eq!(recommend(Some(32), Some(8 * GIB)), 2);
        assert_eq!(recommend(Some(8), Some(32 * GIB)), 4);
        assert_eq!(recommend(Some(10), Some(16 * GIB)), 4);
        assert_eq!(recommend(Some(14), Some(24 * GIB)), 6);
        assert_eq!(recommend(Some(64), Some(64 * GIB)), 16);
        assert_eq!(recommend(None, None), 4);
    }

    #[test]
    fn only_even_slot_counts_are_supported() {
        let supported: Vec<_> = (0..=18).filter(|&lanes| is_supported(lanes)).collect();
        assert_eq!(supported, [2, 4, 6, 8, 10, 12, 14, 16]);
    }
}
