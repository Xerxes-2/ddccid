use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use ddc_hi::{Ddc, Display};

use crate::BrightnessManager;

pub struct DdcHiBackend {
    displays: Vec<Mutex<Display>>,
    last_set: Cell<Instant>,
    last_get: Cell<Instant>,
    temp: Cell<u16>,
}

const COOLDOWN: Duration = Duration::from_millis(200);

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
        let temp = displays[0]
            .lock()
            .unwrap()
            .handle
            .get_vcp_feature(0x10)?
            .value();
        let last = Instant::now();
        Ok(Self {
            displays,
            last_set: Cell::new(last),
            last_get: Cell::new(last),
            temp: Cell::new(temp),
        })
    }

    fn get_brightness(&self) -> Result<u16> {
        if self.last_get.get().elapsed() < COOLDOWN {
            return Ok(self.temp.get());
        }
        let brightness = self.displays[0]
            .lock()
            .unwrap()
            .handle
            .get_vcp_feature(0x10)?
            .value();
        let now = Instant::now();
        self.last_get.set(now);
        self.temp.set(brightness);
        Ok(brightness)
    }

    fn set_brightness(&self, value: u16) -> Result<u16> {
        if self.last_set.get().elapsed() < COOLDOWN * 2 {
            return Ok(self.temp.get());
        }
        let clamped_value = std::cmp::min(100, value);
        std::thread::scope(|s| {
            for d in self.displays.iter() {
                s.spawn(move || {
                    let _ = d
                        .lock()
                        .unwrap()
                        .handle
                        .set_vcp_feature(0x10, clamped_value);
                });
            }
        });
        let now = Instant::now();
        self.last_set.set(now);
        self.temp.set(clamped_value);
        Ok(clamped_value)
    }
}
