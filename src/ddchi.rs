use std::{
    cell::Cell,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use ddc_hi::{Ddc, Display};

use crate::BrightnessManager;

pub struct DdcHiBackend {
    displays: Vec<Mutex<Display>>,
    last_get: Cell<Option<(Instant, u16)>>,
}

pub const GET_COOLDOWN: Duration = Duration::from_millis(100);

impl BrightnessManager for DdcHiBackend {
    type Error = anyhow::Error;
    fn new() -> Result<Self> {
        let displays = Display::enumerate()
            .into_iter()
            .map(Mutex::new)
            .collect::<Vec<_>>();
        if displays.is_empty() {
            bail!("No DDC/CI-capable displays found")
        }
        Ok(Self {
            displays,
            last_get: Cell::new(None),
        })
    }

    fn get_brightness(&self) -> Result<u16> {
        let now = Instant::now();
        if let Some((last_time, last_value)) = self.last_get.get() {
            if now.duration_since(last_time) < GET_COOLDOWN {
                return Ok(last_value);
            }
        }

        let brightness = self.displays[0]
            .lock()
            .unwrap()
            .handle
            .get_vcp_feature(0x10)
            .map(|f| f.value())?;

        self.last_get.set(Some((now, brightness)));
        Ok(brightness)
    }

    fn set_brightness(&self, value: u16) -> Result<u16> {
        let clamped_value = std::cmp::min(100, value);
        self.displays[0]
            .lock()
            .unwrap()
            .handle
            .set_vcp_feature(0x10, clamped_value)?;
        Ok(clamped_value)
    }

    fn adjust_brightness(&self, step: i16) -> Result<u16> {
        let current = self.get_brightness()?;
        let new_value = if step < 0 {
            current.saturating_sub((-step) as u16)
        } else {
            current.saturating_add(step as u16)
        };
        self.set_brightness(new_value)
    }
}
