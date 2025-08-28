#[cfg(feature = "ddc-hi")]
mod ddchi;
#[cfg(feature = "ddcutil")]
mod ddcutil;
#[cfg(feature = "ddc-hi")]
pub use ddchi::DdcHiBackend;
#[cfg(feature = "ddcutil")]
pub use ddcutil::DdcutilBackend;

pub trait BrightnessManager {
    type Error: AsRef<dyn std::error::Error>;
    fn new() -> Result<Self, Self::Error>
    where
        Self: Sized;
    fn get_brightness(&self) -> Result<u16, Self::Error>;
    fn set_brightness(&self, value: u16) -> Result<u16, Self::Error>;
    fn adjust_brightness(&self, step: i16) -> Result<u16, Self::Error>;
}
