use crate::model::{ConnectionRole, ElectronicKey, MAX_ASSEMBLY_BYTES, Settings, hex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod assembly;
pub use assembly::InputLayout;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub fields: BTreeMap<String, String>,
    pub settings: Settings,
    pub unsupported: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_layout: Option<InputLayout>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assembly_definition_sizes: Vec<String>,
}

impl Profile {
    pub fn size_from_assembly_definition(&self, field: &str) -> bool {
        if self
            .assembly_definition_sizes
            .iter()
            .any(|name| name == field)
        {
            return true;
        }
        // Older saved GUI sessions contain the source text but lack the structured marker.
        matches!(field, "O->T size" | "T->O size")
            && self
                .fields
                .get(field)
                .and_then(|source| source.rsplit_once(" ("))
                .and_then(|(_, name)| name.strip_suffix(')'))
                .is_some_and(|name| {
                    name.strip_suffix(" member widths")
                        .unwrap_or(name)
                        .to_ascii_lowercase()
                        .strip_prefix("assem")
                        .is_some_and(|suffix| suffix.parse::<u16>().is_ok())
                })
    }
}

pub fn load(path: &Path) -> Result<Vec<Profile>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        let units = bytes[2..].chunks_exact(2);
        if !units.remainder().is_empty() {
            return Err("Truncated UTF-16 EDS file".into());
        }
        let units: Vec<_> = units
            .map(|pair| {
                let pair = [pair[0], pair[1]];
                if little {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                }
            })
            .collect();
        return parse(&String::from_utf16(&units).map_err(|_| "Invalid UTF-16 EDS file")?);
    }
    parse(&String::from_utf8_lossy(&bytes))
}
fn fields(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => result.push(std::mem::take(&mut field).trim().to_owned()),
            _ => field.push(c),
        }
    }
    result.push(field.trim().to_owned());
    result
}
fn number(text: &str) -> Option<u32> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}
fn path_bytes(text: &str, param_byte: impl Fn(&str) -> Option<u8>) -> Option<Vec<u8>> {
    text.split_whitespace()
        .map(|word| {
            if let Some(parameter) = word
                .strip_prefix('[')
                .and_then(|word| word.strip_suffix(']'))
            {
                return param_byte(parameter);
            }
            let word = word.strip_prefix("0x").unwrap_or(word);
            if word.len() != 2 {
                return None;
            }
            u8::from_str_radix(word, 16).ok()
        })
        .collect()
}

fn assembly_path(mut rest: &[u8]) -> Option<(Option<ElectronicKey>, Option<u16>, u16, u16)> {
    let key = if rest.starts_with(&[0x34, 4]) {
        let (segment, path) = rest.split_at_checked(10)?;
        let key = ElectronicKey {
            vendor: u16::from_le_bytes([segment[2], segment[3]]),
            device_type: u16::from_le_bytes([segment[4], segment[5]]),
            product: u16::from_le_bytes([segment[6], segment[7]]),
            major: segment[8],
            minor: segment[9],
        };
        if key.vendor == 0 || !(1..=127).contains(&key.major) {
            return None;
        }
        rest = path;
        Some(key)
    } else {
        None
    };
    let mut next = |tag: u8| -> Option<u16> {
        match rest {
            [code, n, tail @ ..] if *code == tag => {
                rest = tail;
                Some((*n).into())
            }
            [code, 0, lo, hi, tail @ ..] if *code == (tag | 1) => {
                rest = tail;
                Some(u16::from_le_bytes([*lo, *hi]))
            }
            _ => None,
        }
    };
    if next(0x20)? != 4 {
        return None;
    }
    let config = next(0x24);
    let output = next(0x2c)?;
    let input = next(0x2c)?;
    rest.is_empty().then_some((key, config, output, input))
}
pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let mut cleaned = String::new();
    let mut quoted = false;
    let mut comment = false;
    for c in text.trim_start_matches('\u{feff}').chars() {
        if c == '\n' {
            comment = false;
        }
        if !quoted && c == '$' {
            comment = true;
        }
        if comment {
            continue;
        }
        if c == '"' {
            quoted = !quoted;
        }
        cleaned.push(c);
    }
    if quoted {
        return Err("EDS has an unterminated quoted string".into());
    }
    let mut section = String::new();
    let mut statement = String::new();
    let mut entries = HashMap::new();
    let mut connections = Vec::new();
    for line in cleaned.lines() {
        let line = line.trim();
        if statement.trim().is_empty() && line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        for c in line.chars() {
            if c == '"' {
                quoted = !quoted;
            }
            if c == ';' && !quoted {
                if let Some((name, value)) = statement.split_once('=') {
                    let key = name.trim().to_ascii_lowercase();
                    let value = fields(value);
                    if section == "connection manager"
                        && key
                            .strip_prefix("connection")
                            .is_some_and(|n| n.parse::<u32>().is_ok())
                    {
                        connections.push((name.trim().to_string(), value));
                    } else {
                        entries.insert((section.clone(), key), value);
                    }
                }
                statement.clear();
            } else {
                statement.push(c);
            }
        }
        if !statement.is_empty() {
            statement.push(' ');
        }
    }
    if connections.is_empty() {
        return Err("No ConnectionN profiles found in [Connection Manager]".into());
    }

    let parameter = |text: &str| entries.get(&("params".into(), text.to_ascii_lowercase()));
    let resolve = |text: &str| number(text).or_else(|| number(parameter(text)?.get(11)?));
    let parameter_byte = |name: &str| {
        let fields = parameter(name)?;
        let name = name.to_ascii_lowercase();
        name.strip_prefix("param")?.parse::<u32>().ok()?;
        if number(fields.get(5)?)? != 1 {
            return None;
        }
        u8::try_from(number(fields.get(11)?)?).ok()
    };
    let assembly_definition = |name: &str| {
        let key = name.to_ascii_lowercase();
        key.strip_prefix("assem")?.parse::<u32>().ok()?;
        entries.get(&("assembly".into(), key))
    };
    Ok(connections
        .into_iter()
        .map(|(id, f)| {
            let get = |idx: usize| f.get(idx).map(String::as_str).unwrap_or("");
            let name = get(12).to_string();
            let mut p = Profile {
                id,
                name,
                fields: BTreeMap::new(),
                unsupported: Vec::new(),
                input_layout: None,
                assembly_definition_sizes: Vec::new(),
                settings: Settings {
                    input_instance: 0,
                    output_instance: 0,
                    config_instance: None,
                    input_size: 0,
                    output_size: 0,
                    input_rpi_us: 0,
                    output_rpi_us: 0,
                    input_only: false,
                    ..Settings::default()
                },
            };
            let s = &mut p.settings;
            let origin = &mut p.fields;
            let mut issues = Vec::new();
            let trigger = number(get(0)).unwrap_or(0);
            if trigger & 0x0001_0002 != 0x0001_0002 {
                issues.push("Profile must support cyclic Class 1".into());
            }
            if trigger & 0x0200_0000 != 0 {
                s.set_role(ConnectionRole::InputOnly);
            } else if trigger & 0x0100_0000 != 0 {
                s.set_role(ConnectionRole::ListenOnly);
            } else if trigger & 0x0400_0000 != 0 {
                s.set_role(ConnectionRole::ExclusiveOwner);
            } else {
                issues.push("Profile lacks input-only, listen-only or exclusive-owner support".into());
            }
            if trigger & 0x0700_0000 != 0 {
                origin.insert("Ownership".into(), s.role().label().into());
            }

            let mask = number(get(1)).unwrap_or(0);
            // EDS connection-parameter bits 0/1 describe fixed/variable O->T,
            // and bits 2/3 describe fixed/variable T->O.
            let variable_output = mask & 0x03 == 0x02;
            let variable_input = mask & 0x0c == 0x08;
            if mask & 0x0004_0000 == 0
                || mask & 0x03 == 0
                || mask & 0x0060_0000 == 0
                || mask & 0x0c == 0
            {
                issues.push(
                    "Profile requires point-to-point O->T and point-to-point or multicast T->O with a supported size mode in each direction".into(),
                );
            }
            if variable_output && s.input_only {
                issues.push("Variable-size O->T requires an exclusive-owner profile".into());
            }
            if mask & 0x0060_0000 == 0x0020_0000 {
                s.input_multicast = true;
            }
            if s.input_multicast {
                origin.insert("T->O distribution".into(), "Multicast".into());
            }
            if s.listen_only && mask & 0x0020_0000 == 0 {
                issues.push("Listen-only requires a multicast T->O connection".into());
            }
            for (shift, direction, header, priority) in [
                (0, "O->T", &mut s.output_run_idle, &mut s.output_priority),
                (4, "T->O", &mut s.input_run_idle, &mut s.input_priority),
            ] {
                let format = (mask >> (8 + shift)) & 7;
                let header_label = if shift == 0 {
                    "O->T header"
                } else {
                    "T->O header"
                };
                let priority_label = if shift == 0 {
                    "O->T priority"
                } else {
                    "T->O priority"
                };
                match format {
                    0 => {
                        origin.insert(header_label.into(), "Modeless".into());
                    }
                    3 if shift == 0 && s.input_only => {
                        origin.insert(header_label.into(), "Heartbeat".into());
                    }
                    4 => {
                        *header = true;
                        origin.insert(header_label.into(), "32-bit Run/Idle".into());
                    }
                    _ => issues.push(format!(
                        "{direction} transfer format {format} needs manual review"
                    )),
                }
                let supported = (mask >> (24 + shift)) & 15;
                if supported != 0 {
                    *priority = supported.trailing_zeros() as u8;
                    origin.insert(priority_label.into(), priority.to_string());
                } else {
                    issues.push(format!("{direction} priority missing"));
                }
            }
            if let Some(bytes) = path_bytes(get(14), parameter_byte)
                && let Some((key, config, output, input)) = assembly_path(&bytes)
            {
                s.electronic_key = key;
                if let Some(key) = key {
                    origin.insert(
                        "Electronic key".into(),
                        format!(
                            "{}:{}:{} {}.{}",
                            key.vendor, key.device_type, key.product, key.major, key.minor
                        ),
                    );
                }
                s.config_instance = config;
                s.output_instance = output;
                s.input_instance = input;
                origin.insert(
                    "Config assembly".into(),
                    config.map_or_else(|| "None".into(), |v| v.to_string()),
                );
                origin.insert("O->T assembly".into(), output.to_string());
                origin.insert("T->O assembly".into(), input.to_string());
                if get(14).contains('[') {
                    origin.insert(
                        "Assembly path".into(),
                        format!("{} => {}", get(14), hex(&bytes)),
                    );
                }
            } else {
                issues.push("Unresolved or unsupported assembly path".into());
            }

            for (idx, label, value) in [
                (2, "O->T RPI", &mut s.output_rpi_us),
                (5, "T->O RPI", &mut s.input_rpi_us),
            ] {
                if let Some(n) = resolve(get(idx)) {
                    *value = n;
                    origin.insert(label.into(), format!("{n} µs ({})", get(idx)));
                }
            }
            for (idx, label, value) in [
                (3, "O->T size", &mut s.output_size),
                (6, "T->O size", &mut s.input_size),
            ] {
                let size_field = get(idx);
                let size = if size_field.is_empty() {
                    let format = get(idx + 1);
                    assembly_definition(format)
                        .and_then(|definition| assembly::payload_size(definition, parameter))
                        .map(|(size, from_members)| {
                            let source = if from_members {
                                format!("{format} member widths")
                            } else {
                                format.to_owned()
                            };
                            (size, source)
                        })
                } else {
                    resolve(size_field).map(|size| (size, size_field.to_owned()))
                };
                if let Some((size, source)) = size
                    && let Ok(n) = u16::try_from(size)
                {
                    *value = n;
                    if size_field.is_empty() {
                        p.assembly_definition_sizes.push(label.into());
                    }
                    origin.insert(label.into(), format!("{n} bytes ({source})"));
                }
            }
            if !origin.contains_key("O->T size") {
                issues.push("Unresolved O->T size; verify it in manual mode".into());
            }
            if origin.contains_key("T->O assembly")
                && origin.contains_key("T->O size")
                && let Some(definition) = assembly_definition(get(7))
            {
                p.input_layout = assembly::input_layout(get(7), definition, s.input_size, parameter);
            }
            if variable_output {
                s.output_min_size = Some(s.output_size);
                origin.insert("O->T size mode".into(), "Variable; review minimum output bytes".into());
            }
            if variable_input {
                // The EDS defines the configured maximum, not a reliable local
                // lower bound. Keep the conservative boundary until reviewed.
                s.input_min_size = Some(s.input_size);
                origin.insert("T->O size mode".into(), "Variable; review minimum input bytes".into());
            }
            if usize::from(s.input_size) + 4 * usize::from(s.input_run_idle) > MAX_ASSEMBLY_BYTES
                || usize::from(s.output_size) + 4 * usize::from(s.output_run_idle)
                    > MAX_ASSEMBLY_BYTES
            {
                s.large_forward_open = true;
                origin.insert(
                    "Large Forward_Open".into(),
                    "Required by imported connection size".into(),
                );
            }
            let size1 = if get(8).is_empty() && get(9).is_empty() {
                Some(0)
            } else {
                resolve(get(8))
            };
            let size2 = if get(10).is_empty() && get(11).is_empty() {
                Some(0)
            } else if get(10).is_empty() {
                entries.get(&("assembly".into(), get(11).to_ascii_lowercase()))
                    .and_then(|definition| number(definition.get(2)?))
            } else {
                resolve(get(10))
            };
            match (size1, size2) {
                (Some(0), Some(n)) if n <= 490 => {
                    s.config_size = n as u16;
                    origin.insert("Config size".into(), n.to_string());
                }
                _ => issues.push("Configuration format or size requires manual review".into()),
            }
            p.unsupported = issues;
            p
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_required_variable_output_and_prefers_fixed_when_available() {
        let sample = include_str!("../samples/loopback.eds")
            .replace("0x02010002", "0x04010002")
            .replace("0x22440305", "0x22440406")
            .replace("Param1,0,,", "Param1,12,,")
            .replace("2C FE", "2C 64");
        let profiles = parse(&sample).unwrap();
        let profile = &profiles[0];
        assert!(profile.unsupported.is_empty(), "{:?}", profile.unsupported);
        assert_eq!(profile.settings.output_size, 12);
        assert_eq!(profile.settings.output_min_size, Some(12));
        assert!(profile.settings.output_run_idle);
        assert!(profile.fields.contains_key("O->T size mode"));
        let dual_mode = sample.replace("0x22440406", "0x22440407");
        assert_eq!(parse(&dual_mode).unwrap()[0].settings.output_min_size, None);
    }

    #[test]
    fn sample_and_unresolved_path() {
        let sample = include_str!("../samples/loopback.eds");
        let profiles = parse(sample).unwrap();
        let p = &profiles[0];
        assert!(p.unsupported.is_empty(), "{:?}", p.unsupported);
        assert!(p.settings.input_only);
        assert!(p.size_from_assembly_definition("T->O size"));
        assert!(!p.size_from_assembly_definition("O->T size"));
        let mut legacy = p.clone();
        legacy.assembly_definition_sizes.clear();
        assert!(legacy.size_from_assembly_definition("T->O size"));
        assert_eq!(
            (
                p.settings.input_instance,
                p.settings.input_size,
                p.settings.input_rpi_us
            ),
            (101, 4, 1000)
        );
        let variant = sample.replace("2C 65", "2D 00 [Param2]");
        assert!(!parse(&variant).unwrap()[0].unsupported.is_empty());
    }

    #[test]
    fn derives_cyclic_sizes_from_complete_assembly_members() {
        let sample = include_str!("../samples/loopback.eds")
            .replace(
                "[Assembly]",
                "Param2 = 0,,,0,0xC3,2,\"Temperature\";\n[Assembly]",
            )
            .replace(
                "4, 0x0000,,,32,;",
                ",,,,3,,5,,,Param2,8,;\nAssem102 = \"Output\", \"\", ,0x0001,,,32,;",
            )
            .replace("0x02010002", "0x04010002")
            .replace("0x22440305", "0x22440405")
            .replace("Param1,0,,", "Param1,,Assem102,")
            .replace("2C FE", "2C 66");
        let profiles = parse(&sample).unwrap();
        let profile = &profiles[0];
        assert!(profile.unsupported.is_empty(), "{:?}", profile.unsupported);
        assert_eq!(profile.settings.input_size, 4);
        assert_eq!(profile.settings.output_size, 4);
        assert!(profile.settings.output_run_idle);
        for (direction, format) in [("T->O", "Assem101"), ("O->T", "Assem102")] {
            let field = format!("{direction} size");
            assert!(profile.size_from_assembly_definition(&field));
            assert_eq!(
                profile.fields[&field],
                format!("4 bytes ({format} member widths)")
            );
        }
        let field = &profile.input_layout.as_ref().unwrap().fields[0];
        assert_eq!(field.name, "Temperature");
        assert_eq!(field.selection.offset, 1);
        assert_eq!(field.selection.kind, crate::model::Kind::I16);

        let override_size = sample.replace("Param1,,Assem101", "Param1,8,Assem101");
        let profiles = parse(&override_size).unwrap();
        assert_eq!(profiles[0].settings.input_size, 8);
        assert!(!profiles[0].size_from_assembly_definition("T->O size"));
        assert!(profiles[0].input_layout.is_none());

        for invalid in [
            sample.replace("3,,5,,,", "3,,4,,,"),
            sample.replace(",Param2,8,", ",Param99,8,"),
            sample.replace("0xC3,2,", "0xC3,4,"),
            sample.replace("8,;", "8;"),
            sample.replace("8,;", "524288,;"),
            sample.replace("Param1,,Assem101", "Param1,unknown,Assem101"),
        ] {
            let profiles = parse(&invalid).unwrap();
            assert!(!profiles[0].fields.contains_key("T->O size"));
            assert!(profiles[0].input_layout.is_none());
        }
    }

    #[test]
    fn resolves_path_byte_parameters_without_truncation() {
        let sample = include_str!("../samples/loopback.eds")
            .replace("[Assembly]", "Param5 = 0,,,0,0xC6,1,\"Input\",\"\",\"\",101,101,101;\nParam7 = 0,,,0,0xC6,1,\"Heartbeat\",\"\",\"\",254,254,254;\n[Assembly]")
            .replace("0x22440305", "0x44244305")
            .replace("2C FE 2C 65", "2C [Param7] 2C [Param5]");
        let profiles = parse(&sample).unwrap();
        let p = &profiles[0];
        // The profile requires multicast T->O and retains the decoded path.
        assert_eq!(p.settings.config_instance, Some(1));
        assert_eq!(p.settings.output_instance, 254);
        assert_eq!(p.settings.input_instance, 101);
        assert!(p.settings.input_multicast);
        assert_eq!(p.fields["T->O distribution"], "Multicast");
        assert!(p.unsupported.is_empty(), "{:?}", p.unsupported);
        assert_eq!(
            p.fields["Assembly path"],
            "20 04 24 01 2C [Param7] 2C [Param5] => 20 04 24 01 2C FE 2C 65"
        );
        let too_wide = sample.replace("254,254,254", "254,254,510");
        assert!(
            parse(&too_wide).unwrap()[0]
                .unsupported
                .iter()
                .any(|e| e.contains("assembly path"))
        );
    }
}
