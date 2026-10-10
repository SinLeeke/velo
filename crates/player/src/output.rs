//! Generación de cambio de salida; el sink la aplica en start/write sin cambiar el decoder.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[derive(Clone)]
pub(crate) struct OutputSwitch(Arc<OutputOptions>);
struct OutputOptions {
    generation: AtomicU64,
    buffer_ms: AtomicU64,
}
impl Default for OutputSwitch {
    fn default() -> Self {
        Self(Arc::new(OutputOptions {
            generation: AtomicU64::new(0),
            buffer_ms: AtomicU64::new(100),
        }))
    }
}
impl OutputSwitch {
    pub fn request(&self) {
        self.0.generation.fetch_add(1, Ordering::Release);
    }
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }
    pub fn set_buffer_ms(&self, ms: u32) -> Result<(), crate::PlayerError> {
        if !matches!(ms, 50 | 100 | 200) {
            return Err(crate::PlayerError::InvalidOptions);
        }
        if self.0.buffer_ms.swap(u64::from(ms), Ordering::AcqRel) != u64::from(ms) {
            self.request();
        }
        Ok(())
    }
    pub fn buffer_ms(&self) -> u32 {
        self.0.buffer_ms.load(Ordering::Acquire) as u32
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_share_generation() {
        let switch = OutputSwitch::default();
        let copy = switch.clone();
        switch.request();
        switch.request();
        assert_eq!(copy.generation(), 2);
    }
    #[test]
    fn buffer_changes_reopen_only_when_valid_and_different() {
        let switch = OutputSwitch::default();
        assert_eq!(switch.buffer_ms(), 100);
        switch.set_buffer_ms(50).unwrap();
        switch.set_buffer_ms(50).unwrap();
        assert_eq!(switch.generation(), 1);
        assert!(switch.set_buffer_ms(0).is_err());
        assert_eq!(switch.buffer_ms(), 50);
        assert_eq!(switch.generation(), 1);
    }
}
