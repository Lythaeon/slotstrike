use crate::domain::settings::NetworkStackMode;

pub trait NetworkPathPort: Send + Sync {
    fn mode(&self) -> NetworkStackMode;
    fn describe(&self) -> String;
    fn kernel_bypass_enabled(&self) -> bool;
    fn fpga_enabled(&self) -> bool;
}
