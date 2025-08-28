use super::BrightnessManager;
use anyhow::bail;
use ddcutil::{Display, DisplayInfo};
use itertools::Itertools;

pub struct DdcutilBackend {
    displays: Vec<Display>,
}

impl BrightnessManager for DdcutilBackend {
    type Error = anyhow::Error;

    fn new() -> Result<Self, Self::Error> {
        let displays = DisplayInfo::enumerate()?;
        if displays.is_empty() {
            bail!("No DDC/CI-capable displays found")
        }
        let displays = displays.into_iter().map(|info| info.open()).try_collect()?;
        Ok(Self { displays })
    }

    fn get_brightness(&self) -> Result<u16, Self::Error> {
        let brightness = self.displays[0].vcp_get_value(0x10)?;
        Ok(brightness.value())
    }

    fn set_brightness(&self, value: u16) -> Result<u16, Self::Error> {
        let clamped_value = std::cmp::min(100, value);
        for display in &self.displays {
            display.vcp_set_raw(0x10, clamped_value)?;
        }
        Ok(clamped_value)
    }

    fn adjust_brightness(&self, step: i16) -> Result<u16, Self::Error> {
        let current = self.get_brightness()?;
        let new_value = if step < 0 {
            current.saturating_sub((-step) as u16)
        } else {
            current.saturating_add(step as u16)
        };
        self.set_brightness(new_value)
    }
}
