//! Power limits from lenovo-wmi-other's firmware attributes (kernel 6.16+):
//! /sys/class/firmware-attributes/lenovo-wmi-other-N/attributes/<name>/{current_value,min_value,max_value,...}
//! The firmware takes them in Custom mode, so applying switches to Custom first. Saved values (config
//! "fw_<name>") are applied again whenever Custom is selected, like the Windows Custom mode.

use std::path::Path;

use crate::config;

const ROOT: &str = "/sys/class/firmware-attributes";

#[derive(Debug, Clone)]
pub struct Attribute {
    pub class: String,
    pub name: String,
    pub label: String,
    pub value: i64,
    pub min: i64,
    pub max: i64,
    pub step: i64,
}

impl Attribute {
    pub fn config_key(&self) -> String {
        format!("fw_{}", self.name)
    }

    /// Unit for the known power limits
    pub fn unit(&self) -> &'static str {
        if self.name.starts_with("ppt_") || self.name.contains("tgp") || self.name.contains("power") {
            " W"
        } else if self.name.contains("temp") {
            " °C"
        } else {
            ""
        }
    }
}

/// The gaming firmware's class only: ThinkPads list hundreds of BIOS settings in another class (think-lmi)
pub fn is_lenovo_class(name: &str) -> bool {
    name.starts_with("lenovo-wmi-other")
}

fn friendly(name: &str) -> Option<&'static str> {
    Some(match name {
        "ppt_pl1_spl" => "CPU sustained power (PL1)",
        "ppt_pl2_sppt" => "CPU boost power (PL2)",
        "ppt_pl3_fppt" => "CPU peak power (PL3)",
        "ppt_pl1_tau" => "CPU boost duration",
        "gpu_nv_ppab" => "GPU dynamic boost",
        "gpu_nv_ctgp" => "GPU configurable TGP",
        "gpu_nv_cpu_boost" => "GPU power when CPU boosts",
        "gpu_temp" | "gpu_temp_limit" => "GPU temperature limit",
        "cpu_temp" | "cpu_temp_limit" => "CPU temperature limit",
        _ => return None,
    })
}

fn read_attribute(class: &str, dir: &Path) -> Option<Attribute> {
    let name = dir.file_name()?.to_str()?.to_string();
    let kind = super::read(dir.join("type"));
    if kind.as_deref().is_some_and(|k| k != "integer") {
        return None;
    }
    let (value, min, max) = (super::read_i64(dir.join("current_value"))?, super::read_i64(dir.join("min_value"))?, super::read_i64(dir.join("max_value"))?);
    if min >= max {
        return None;
    }
    let step = super::read_i64(dir.join("scalar_increment")).filter(|s| *s > 0).unwrap_or(1);
    let label = friendly(&name).map(str::to_string).or_else(|| super::read(dir.join("display_name")).filter(|s| !s.is_empty())).unwrap_or_else(|| name.clone());
    Some(Attribute { class: class.to_string(), name, label, value, min, max, step })
}

pub fn attributes() -> Vec<Attribute> {
    let mut out = Vec::new();
    for class in super::dirs(ROOT) {
        let Some(class_name) = class.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        if !is_lenovo_class(&class_name) {
            continue;
        }
        out.extend(super::dirs(class.join("attributes")).iter().filter_map(|d| read_attribute(&class_name, d)));
    }
    out
}

pub fn supported() -> bool {
    !attributes().is_empty()
}

pub fn set(a: &Attribute, value: i64) -> Result<(), String> {
    let value = value.clamp(a.min, a.max);
    crate::log!("  {} = {value}", a.name);
    super::privileged(&["attribute", &a.class, &a.name, &value.to_string()])
}

/// Saved Custom values, written where the firmware currently differs
pub fn apply_saved() -> Result<(), String> {
    let mut errors = Vec::new();
    for a in attributes() {
        let key = a.config_key();
        if !config::exists(&key) {
            continue;
        }
        let saved = config::get(&key, a.value).clamp(a.min, a.max);
        if saved != a.value {
            if let Err(e) = set(&a, saved) {
                errors.push(format!("{}: {e}", a.label));
            }
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}
