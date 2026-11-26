use serde::Serialize;

pub const MIN: usize = 2;
pub const DEFAULT: usize = 8;
pub const MAX: usize = 16;

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
    pub fn from_capacity(capacity: Option<usize>) -> Self {
        let capacity = capacity.unwrap_or(DEFAULT).clamp(MIN, MAX);
        Self {
            logical_cores: None,
            memory_bytes: None,
            recommended_concurrency: capacity - capacity % 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_concurrency_uses_the_gpu_server_capacity() {
        assert_eq!(
            HardwareProfile::from_capacity(None).recommended_concurrency,
            8
        );
        assert_eq!(
            HardwareProfile::from_capacity(Some(7)).recommended_concurrency,
            6
        );
        assert_eq!(
            HardwareProfile::from_capacity(Some(128)).recommended_concurrency,
            16
        );
        assert_eq!(
            HardwareProfile::from_capacity(Some(0)).recommended_concurrency,
            2
        );
    }

    #[test]
    fn only_even_slot_counts_are_supported() {
        let supported: Vec<_> = (0..=18).filter(|&lanes| is_supported(lanes)).collect();
        assert_eq!(supported, [2, 4, 6, 8, 10, 12, 14, 16]);
    }
}
