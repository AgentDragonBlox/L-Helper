//! Custom-mode fan curve: LENOVO_FAN_TABLE_DATA (per-fan RPM / temperature points) and LENOVO_FAN_METHOD.Fan_Set_Table.
//! One 10-level table drives all fans; level n maps to fan_speeds[n] RPM at temps[i] (as in LLT).

use super::gamezone::{self, Cap};
use super::machine;
use crate::wmi::{self, Arg};

const METHOD: &str = "LENOVO_FAN_METHOD";

pub const DEFAULT_TABLE: [i32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
pub const MINIMUM_TABLE: [i32; 10] = [1, 1, 1, 1, 1, 1, 1, 1, 3, 5];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FanKind {
    Cpu,
    Gpu,
    Pch,
}

#[derive(Debug, Clone)]
pub struct FanTableData {
    pub kind: FanKind,
    pub fan_id: u8,
    pub sensor_id: u8,
    pub speeds: Vec<i32>,
    pub temps: Vec<i32>,
}

/// Fan / sensor id pairs per LLT GodModeController.ResolveFanTableType
fn resolve(fan: u8, sensor: u8) -> Option<FanKind> {
    let m = machine::get();
    if (1..=7).contains(&m.smart_fan_version) {
        return match (fan, sensor) {
            (1, 4) | (0, 3) => Some(FanKind::Cpu),
            (2, 5) | (3, 5) => Some(FanKind::Gpu),
            _ => None,
        };
    }
    if m.is_v3_fan_layout() {
        return match (fan, sensor) {
            (1, 1) => Some(FanKind::Cpu),
            (2, 5) => Some(FanKind::Gpu),
            (1, 4) => Some(FanKind::Pch),
            _ => None,
        };
    }
    match (fan, sensor) {
        (1, 1) | (1, 4) => Some(FanKind::Cpu),
        (2, 5) => Some(FanKind::Gpu),
        (4, 4) | (5, 5) | (4, 1) => Some(FanKind::Pch),
        _ => None,
    }
}

pub fn read_table_data() -> Vec<FanTableData> {
    let rows = match wmi::query("SELECT * FROM LENOVO_FAN_TABLE_DATA") {
        Ok(r) => r,
        Err(e) => {
            crate::log!("Can't read fan table data: {e}");
            return Vec::new();
        }
    };
    let rows: Vec<(i64, u8, u8, Vec<i32>, Vec<i32>)> = rows
        .iter()
        .map(|r| {
            (
                r.get("Mode").as_i64().unwrap_or(-1),
                r.get("Fan_Id").as_i64().unwrap_or(0) as u8,
                r.get("Sensor_ID").as_i64().unwrap_or(0) as u8,
                r.get("FanTable_Data").as_array().into_iter().map(|v| v as i32).collect(),
                r.get("SensorTable_Data").as_array().into_iter().map(|v| v as i32).collect(),
            )
        })
        .collect();

    crate::log!("Fan table rows: {}", rows.iter().map(|r| format!("mode={} fan={} sensor={} n={}", r.0, r.1, r.2, r.3.len())).collect::<Vec<_>>().join(" | "));

    // Gen 8+ firmware has one row set per power mode, Custom = 255
    let custom = rows.iter().any(|r| r.0 == 255);
    let mut result: Vec<FanTableData> = Vec::new();
    for (mode, fan, sensor, speeds, temps) in rows {
        if custom && mode != 255 {
            continue;
        }
        let Some(kind) = resolve(fan, sensor) else { continue };
        if speeds.len() != 10 || temps.len() != 10 || result.iter().any(|d| d.kind == kind) {
            continue;
        }
        result.push(FanTableData { kind, fan_id: fan, sensor_id: sensor, speeds, temps });
    }
    result.sort_by_key(|d| d.kind);
    crate::log!(
        "Fan tables: {}",
        result.iter().map(|d| format!("{:?} fan={} sensor={} rpm={:?} t={:?}", d.kind, d.fan_id, d.sensor_id, d.speeds, d.temps)).collect::<Vec<_>>().join(" | ")
    );
    result
}

/// LLT's legacy-platform minimum: level 0 on the low points. Tested on a LOQ 15AHP10 (BIOS R8CN26WW): the firmware accepts
/// this table, but rejects an all-zero one (it substitutes [1,2,3,4,5,6,7,7,7,7]).
pub const ZERO_MINIMUM_TABLE: [i32; 10] = [0, 0, 0, 0, 0, 0, 0, 1, 3, 5];

/// "Allow fans to stop": level 0 allowed on the low points. Off by default: then the floor is LLT's current minimum table.
pub fn allow_zero() -> bool {
    crate::config::is("fan_allow_zero")
}

pub fn minimum_table() -> [i32; 10] {
    if allow_zero() { ZERO_MINIMUM_TABLE } else { MINIMUM_TABLE }
}

pub fn sanitize(table: &[i32]) -> [i32; 10] {
    if table.len() != 10 {
        return DEFAULT_TABLE;
    }
    let minimum = minimum_table();
    let mut out = [0; 10];
    for i in 0..10 {
        out[i] = table[i].clamp(minimum[i], 10);
        if i > 0 && out[i] < out[i - 1] {
            out[i] = out[i - 1];
        }
    }
    out
}

fn to_bytes(table: &[i32; 10]) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    b.push(1); // FSTM
    b.push(0); // FSID
    b.extend_from_slice(&10u32.to_le_bytes()); // FSTL
    for level in table {
        b.extend_from_slice(&(*level as u16).to_le_bytes());
    }
    b.resize(64, 0);
    b
}

pub fn get_table(fan: u8, sensor: u8) -> Option<Vec<i32>> {
    let out = wmi::call(METHOD, "Fan_Get_Table", &[("FanID", Arg::U8(fan)), ("SensorID", Arg::U8(sensor))]).ok()??;
    let size = out.i32("FanTableSize").unwrap_or(10) as usize;
    Some(out.get("FanTable").as_array().into_iter().take(size).map(|v| v as i32).collect())
}

/// Writes the custom fan table and reads it back; Err describes what the firmware kept instead
pub fn set_table(table: &[i32]) -> Result<(), String> {
    let table = sanitize(table);
    let mut last = String::new();
    for attempt in 1..=3 {
        match wmi::call(METHOD, "Fan_Set_Table", &[("FanTable", Arg::Bytes(to_bytes(&table)))]) {
            Ok(_) => {
                let back = get_table(1, 1);
                if back.as_ref().map(|b| b.len() >= 10 && b[..10] == table[..]).unwrap_or(true) {
                    crate::log!("Fan table set: {table:?}");
                    return Ok(());
                }
                crate::log!("Fan table verify mismatch (attempt {attempt}): {back:?}");
                last = format!("The firmware didn't keep the fan curve {table:?}; it reads back {:?}", back.unwrap_or_default());
            }
            Err(e) => {
                crate::log!("Fan table set failed (attempt {attempt}): {e}");
                last = format!("Fan curve write failed: {e}");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(last)
}

pub fn full_speed() -> Option<bool> {
    if let Some(v) = gamezone::try_feature(Cap::FanFullSpeed) {
        return Some(v != 0);
    }
    wmi::call(METHOD, "Fan_Get_FullSpeed", &[]).ok().flatten().and_then(|o| o.get("Status").as_bool())
}

pub fn set_full_speed(enabled: bool) -> Result<(), String> {
    if gamezone::set_feature(Cap::FanFullSpeed, enabled as i32).is_err() {
        wmi::call(METHOD, "Fan_Set_FullSpeed", &[("Status", Arg::I32(enabled as i32))])?;
    }
    crate::log!("Fan full speed: {enabled}");
    Ok(())
}
