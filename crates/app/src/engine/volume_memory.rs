//! Remember audible volume before watch channels can coalesce successive UI states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeMemory(i32);

impl Default for VolumeMemory {
    fn default() -> Self {
        Self(50)
    }
}

impl VolumeMemory {
    pub fn observe(&mut self, volume: i32) {
        if volume > 0 {
            self.0 = volume.min(100);
        }
    }
    pub fn audible(self) -> i32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mute_keeps_latest_audible_value_even_before_ui_observes_it() {
        let mut memory = VolumeMemory::default();
        assert_eq!(memory.audible(), 50);
        memory.observe(27);
        memory.observe(0);
        memory.observe(-1);
        assert_eq!(memory.audible(), 27);
        memory.observe(83);
        memory.observe(0);
        assert_eq!(memory.audible(), 83);
        memory.observe(200);
        assert_eq!(memory.audible(), 100);
    }
}
