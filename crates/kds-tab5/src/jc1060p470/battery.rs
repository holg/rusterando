//! The JC1060P470 runs on its power supply: no battery to read or guard.
//! The same interface as the Tab5's battery.rs, always "no pack", so the
//! Start page shows no battery line and no "Akku schonen" button.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use crate::ui::UiMsg;

#[derive(Clone, Copy, Default, PartialEq)]
pub struct BatteryStatus;

impl BatteryStatus {
    pub fn describe(&self) -> Option<String> {
        None
    }
}

pub type SharedBattery = Arc<Mutex<Option<BatteryStatus>>>;

#[derive(Clone)]
pub struct Care;

impl Care {
    pub fn load(_nvs: EspDefaultNvsPartition) -> Self {
        Self
    }

    pub fn enabled(&self) -> bool {
        false
    }

    pub fn toggle(&self) -> bool {
        false
    }
}

pub fn start(_status: SharedBattery, _care: Care, _ui: Sender<UiMsg>) -> Result<()> {
    Ok(())
}
