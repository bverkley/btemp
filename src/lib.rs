//! btemp: lm-sensors temperature TUI.

pub mod cpu_info;
pub mod group;
pub mod history;
pub mod sensors;
pub mod storage_names;
pub mod ui;

pub use ui::run;
