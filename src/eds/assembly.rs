use super::number;
use crate::model::{CAPTURE_METADATA, CaptureField, Kind, Selection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Raw mapping candidates from the assembly format named by the connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputLayout {
    pub format: String,
    pub fields: Vec<CaptureField>,
    pub skipped_members: usize,
}

pub(super) fn input_layout<'a>(
    format: &str,
    definition: &[String],
    input_size: u16,
    parameter: impl Fn(&str) -> Option<&'a Vec<String>>,
) -> Option<InputLayout> {
    let (header, members) = definition.split_at_checked(6)?;
    let size = number(&header[2])?;
    // Restrict inference to a plain assembly with a known payload width.
    if size != u32::from(input_size) || number(&header[3])? != 0 || members.is_empty() {
        return None;
    }
    let mut layout = InputLayout {
        format: format.into(),
        fields: Vec::new(),
        skipped_members: 0,
    };
    let mut names: HashSet<String> = CAPTURE_METADATA
        .into_iter()
        .chain(["value"])
        .map(str::to_ascii_lowercase)
        .collect();
    let mut offset = 0u32;
    for pair in members.chunks(2) {
        let [width, reference] = pair else {
            return None;
        };
        let bits = number(width)?;
        let end = offset.checked_add(bits)?;
        if bits == 0 || end > size * 8 {
            return None;
        }
        if !reference.is_empty() {
            match parameter_field(reference, offset, bits, &parameter) {
                Some((name, selection)) => {
                    let name = mapping_name(&name, &mut names);
                    layout.fields.push(CaptureField { name, selection });
                }
                None => layout.skipped_members += 1,
            }
        }
        offset = end;
    }
    Some(layout)
}

fn parameter_field<'a>(
    reference: &str,
    offset: u32,
    bits: u32,
    parameter: &impl Fn(&str) -> Option<&'a Vec<String>>,
) -> Option<(String, Selection)> {
    reference
        .to_ascii_lowercase()
        .strip_prefix("param")?
        .parse::<u32>()
        .ok()?;
    let definition = parameter(reference)?;
    let (kind, bytes) = match number(definition.get(4)?)? {
        0xc2 => (Kind::I8, 1),
        0xc3 => (Kind::I16, 2),
        0xc4 => (Kind::I32, 4),
        0xc5 => (Kind::I64, 8),
        0xc6 | 0xd1 => (Kind::U8, 1),
        0xc7 | 0xd2 => (Kind::U16, 2),
        0xc8 | 0xd3 => (Kind::U32, 4),
        0xc9 | 0xd4 => (Kind::U64, 8),
        0xca => (Kind::F32, 4),
        0xcb => (Kind::F64, 8),
        _ => return None,
    };
    if offset % 8 != 0 || bits != bytes * 8 || number(definition.get(5)?)? != bytes {
        return None;
    }
    let name = definition.get(6)?.trim();
    let name = if name.is_empty() { reference } else { name };
    Some((
        name.to_owned(),
        Selection {
            offset: (offset / 8) as usize,
            kind,
            ..Selection::default()
        },
    ))
}

fn mapping_name(name: &str, used: &mut HashSet<String>) -> String {
    let mut name = name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert_str(0, "input_");
    }
    if used.insert(name.to_ascii_lowercase()) {
        return name;
    }
    for index in 2.. {
        let candidate = format!("{name}_{index}");
        if used.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_uses_declared_offsets_and_types_only() {
        let source = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                concat!(
                    "Param2 = 0,,,0,0xC3,2,\"Net weight\";\n",
                    "Param3 = 0,,,0,0xCA,4,\"Ratio\";\n",
                    "Param4 = 0,,,0,0xD0,2,\"Text\";\n",
                    "Param5 = 0,,,0,0xD1,1,\"net weight\";\n",
                    "[Assembly]"
                ),
            )
            .replace(
                "4, 0x0000,,,32,;",
                "20, 0x0000,,,16,Param2,8,,16,Param4,32,Param3,8,Param5,8,Param2,1,Param5,8,Param5,7,,8,Param5;",
            );
        let mut profiles = super::super::parse(&source).unwrap();
        let layout = profiles[0].input_layout.take().unwrap();
        assert_eq!(layout.format, "Assem101");
        assert_eq!(layout.skipped_members, 4);
        let fields: Vec<_> = layout
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.as_str(),
                    field.selection.offset,
                    field.selection.kind,
                )
            })
            .collect();
        assert_eq!(
            fields,
            [
                ("Net_weight", 0, Kind::I16),
                ("Ratio", 5, Kind::F32),
                ("net_weight_2", 9, Kind::U8),
                ("net_weight_3", 13, Kind::U8)
            ]
        );
        for invalid in [
            source.replace("16,Param2,8,,", "16,Param2,,"),
            source.replace("16,Param2,8,,", "160,Param2,8,,"),
            source.replace("Param1,,Assem101", "Param1,21,Assem101"),
        ] {
            let parsed = super::super::parse(&invalid).unwrap();
            assert!(parsed[0].input_layout.is_none());
            assert!(parsed[0].unsupported.is_empty());
        }
    }
}
