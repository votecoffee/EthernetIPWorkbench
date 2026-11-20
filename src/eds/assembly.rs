use super::number;
use crate::model::{CAPTURE_METADATA, CaptureField, Kind, Selection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, hash_map::Entry};

const CIP_BOOL: u32 = 0xc1;
const MAX_NESTING: usize = 32;

/// Raw mapping candidates from the assembly format named by the connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyLayout {
    pub format: String,
    pub fields: Vec<CaptureField>,
    pub skipped_members: usize,
}

pub(super) struct LayoutReader<'a> {
    entries: &'a HashMap<(String, String), Vec<String>>,
    // A pending entry detects cycles while determining omitted sizes.
    sizes: HashMap<String, Option<(u32, bool)>>,
}

struct Member<'a> {
    offset: u32,
    bits: u32,
    reference: &'a str,
}

impl<'a> LayoutReader<'a> {
    pub(super) fn new(entries: &'a HashMap<(String, String), Vec<String>>) -> Self {
        Self {
            entries,
            sizes: HashMap::new(),
        }
    }

    pub(super) fn payload_size(&mut self, format: &str) -> Option<(u32, bool)> {
        self.size_at(format, 0)
    }

    fn size_at(&mut self, format: &str, depth: usize) -> Option<(u32, bool)> {
        let key = numbered_key(format, "assem")?;
        if let Some(&size) = self.sizes.get(&key) {
            return size;
        }
        let definition = self.definition("assembly", &key)?;
        let declared = definition.get(2)?;
        if !declared.is_empty() {
            return Some((number(declared)?, false));
        }
        if depth >= MAX_NESTING {
            return None;
        }
        self.sizes.insert(key.clone(), None);
        let size = self
            .members(definition, depth)
            .and_then(|(_, bits)| (bits > 0 && bits % 8 == 0).then_some((bits / 8, true)));
        if size.is_some() {
            self.sizes.insert(key, size);
        } else {
            self.sizes.remove(&key);
        }
        size
    }

    pub(super) fn mapping_layout(
        &mut self,
        format: &str,
        payload_size: u16,
    ) -> Option<AssemblyLayout> {
        let (size, _) = self.payload_size(format)?;
        if size != u32::from(payload_size) {
            return None;
        }
        let mut layout = AssemblyLayout {
            format: format.into(),
            fields: Vec::new(),
            skipped_members: 0,
        };
        let mut names = CAPTURE_METADATA
            .into_iter()
            .chain(["value"])
            .map(|name| (name.to_ascii_lowercase(), 2))
            .collect();
        self.collect_fields(
            format,
            0,
            size * 8,
            &mut Vec::new(),
            &mut layout,
            &mut names,
        )?;
        Some(layout)
    }

    fn collect_fields(
        &mut self,
        format: &str,
        start: u32,
        bits: u32,
        ancestors: &mut Vec<String>,
        layout: &mut AssemblyLayout,
        names: &mut HashMap<String, usize>,
    ) -> Option<()> {
        let key = numbered_key(format, "assem")?;
        if ancestors.len() >= MAX_NESTING || ancestors.contains(&key) {
            return None;
        }
        let definition = self.definition("assembly", &key)?;
        let (members, used_bits) = self.members(definition, ancestors.len())?;
        if used_bits > bits {
            return None;
        }
        ancestors.push(key);
        for member in members {
            let reference = member.reference;
            if reference.is_empty() {
                continue;
            }
            let offset = start + member.offset;
            if let Some(parameter) = self.parameter(reference)
                && let Some((name, selection)) =
                    parameter_field(parameter, reference, offset, member.bits)
            {
                let name = mapping_name(&name, names);
                layout.fields.push(CaptureField { name, selection });
                continue;
            }
            // The outer member's width accounts for any trailing padding.
            if let Some(nested_bits) = self.assembly_bits(reference, ancestors.len())
                && nested_bits <= member.bits
                && self
                    .collect_fields(reference, offset, nested_bits, ancestors, layout, names)
                    .is_some()
            {
                continue;
            }
            layout.skipped_members += 1;
        }
        ancestors.pop();
        Some(())
    }

    fn members(
        &mut self,
        definition: &'a [String],
        depth: usize,
    ) -> Option<(Vec<Member<'a>>, u32)> {
        let pairs = member_pairs(definition)?;
        let mut members = Vec::with_capacity(pairs.len());
        let mut offset = 0u32;
        for [width, reference] in pairs {
            let bits = if !width.is_empty() {
                number(width)?
            } else if let Some(parameter) = self.parameter(reference) {
                parameter_type(parameter)?.1 * 8
            } else {
                self.assembly_bits(reference, depth + 1)?
            };
            if bits == 0 {
                return None;
            }
            members.push(Member {
                offset,
                bits,
                reference,
            });
            offset = offset.checked_add(bits)?;
        }
        Some((members, offset))
    }

    fn assembly_bits(&mut self, format: &str, depth: usize) -> Option<u32> {
        let (size, _) = self.size_at(format, depth)?;
        size.checked_mul(8).filter(|&bits| bits > 0)
    }

    fn parameter(&self, reference: &str) -> Option<&'a [String]> {
        self.definition("params", &numbered_key(reference, "param")?)
    }

    fn definition(&self, section: &str, key: &str) -> Option<&'a [String]> {
        self.entries
            .get(&(section.into(), key.into()))
            .map(Vec::as_slice)
    }
}

fn numbered_key(reference: &str, prefix: &str) -> Option<String> {
    let key = reference.to_ascii_lowercase();
    key.strip_prefix(prefix)?.parse::<u32>().ok()?;
    Some(key)
}

fn member_pairs(definition: &[String]) -> Option<&[[String; 2]]> {
    let (header, members) = definition.split_at_checked(6)?;
    // Bit 0 allows editing constant values and leaves the width unchanged.
    if (!header[3].is_empty() && !matches!(number(&header[3]), Some(0 | 1)))
        || !header[4].is_empty()
        || !header[5].is_empty()
    {
        return None;
    }
    let (pairs, trailing) = members.as_chunks::<2>();
    (!pairs.is_empty() && trailing.is_empty()).then_some(pairs)
}

fn parameter_field(
    definition: &[String],
    reference: &str,
    offset: u32,
    bits: u32,
) -> Option<(String, Selection)> {
    let byte_size = definition.get(5)?;
    let (kind, bit) = if bits == 1
        && number(definition.get(4)?) == Some(CIP_BOOL)
        && (byte_size.is_empty() || number(byte_size) == Some(1))
    {
        (Kind::Bool, Some((offset % 8) as u8))
    } else {
        let (kind, bytes) = parameter_type(definition)?;
        if offset % 8 != 0 || bits != bytes * 8 {
            return None;
        }
        (kind, None)
    };
    let name = definition.get(6)?.trim();
    let name = if name.is_empty() { reference } else { name };
    Some((
        name.to_owned(),
        Selection {
            offset: (offset / 8) as usize,
            kind,
            bit,
            ..Selection::default()
        },
    ))
}

fn parameter_type(definition: &[String]) -> Option<(Kind, u32)> {
    let (kind, bytes) = match number(definition.get(4)?)? {
        CIP_BOOL => (Kind::Bool, 1),
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
    (number(definition.get(5)?)? == bytes).then_some((kind, bytes))
}

fn mapping_name(name: &str, used: &mut HashMap<String, usize>) -> String {
    let mut name = name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert_str(0, "input_");
    }
    let key = name.to_ascii_lowercase();
    if let Entry::Vacant(entry) = used.entry(key.clone()) {
        entry.insert(2);
        return name;
    }
    for index in used[&key].. {
        let candidate = format!("{name}_{index}");
        if let Entry::Vacant(entry) = used.entry(candidate.to_ascii_lowercase()) {
            entry.insert(2);
            used.insert(key, index + 1);
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Value;

    #[test]
    fn nested_assemblies_flatten_repeated_fields_and_padding() {
        let source = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                concat!(
                    "Param2 = 0,,,0,0xC1,,\"Ready\";\n",
                    "Param3 = 0,,,0,0xC2,1,\"Limit\";\n",
                    "Param4 = 0,,,0,0xC6,1,\"Counter\";\n",
                    "[Assembly]"
                ),
            )
            .replace(
                "4, 0x0000,,,32,;",
                ",0x0000,,,8,Param4,,Assem201,24,aSSeM201,8,Assem201,8,Param4;",
            )
            .replace(
                "[Connection Manager]",
                concat!(
                    "Assem201 = \"Group\",,2,0,,, ,Assem202,8,Param3;\n",
                    "Assem202 = \"Status\",,,0,,,3,,1,Param2,4,;\n",
                    "[Connection Manager]"
                ),
            );
        let profiles = super::super::parse(&source).unwrap();
        let profile = &profiles[0];
        assert_eq!(profile.settings.input_size, 8);
        assert!(profile.size_from_assembly_definition("T->O size"));
        let layout = profile.input_layout.as_ref().unwrap();
        assert_eq!(layout.skipped_members, 1);
        let bytes = [1, 8, 0xf9, 0, 7, 0xff, 0xff, 0x21];
        let fields: Vec<_> = layout
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.as_str(),
                    field.selection.offset,
                    field.selection.bit,
                    field.selection.decode(&bytes),
                )
            })
            .collect();
        assert_eq!(
            fields,
            [
                ("Counter", 0, None, Some(Value::Unsigned(1))),
                ("Ready", 1, Some(3), Some(Value::Bool(true))),
                ("Limit", 2, None, Some(Value::Signed(-7))),
                ("Ready_2", 3, Some(3), Some(Value::Bool(false))),
                ("Limit_2", 4, None, Some(Value::Signed(7))),
                ("Counter_2", 7, None, Some(Value::Unsigned(33))),
            ]
        );
    }

    #[test]
    fn unresolved_or_circular_nested_assemblies_use_only_known_widths() {
        let sample = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                "Param2 = 0,,,0,0xC6,1,\"Counter\";\n[Assembly]",
            )
            .replace(
                "[Connection Manager]",
                "Assem201 = \"Circular\",,,0,,, ,ASSEM101;\n[Connection Manager]",
            );
        for reference in ["Assem201", "Assem999"] {
            let unresolved = sample.replace(
                "4, 0x0000,,,32,;",
                &format!(",0x0000,,, ,{reference},8,Param2;"),
            );
            let profiles = super::super::parse(&unresolved).unwrap();
            assert!(!profiles[0].fields.contains_key("T->O size"), "{reference}");
            assert!(profiles[0].input_layout.is_none());

            let sized = unresolved.replace(
                &format!("0x0000,,, ,{reference}"),
                &format!("0x0000,,,24,{reference}"),
            );
            let profiles = super::super::parse(&sized).unwrap();
            let layout = profiles[0].input_layout.as_ref().unwrap();
            assert_eq!(profiles[0].settings.input_size, 4);
            assert_eq!(layout.skipped_members, 1);
            assert_eq!(layout.fields.len(), 1);
            assert_eq!(layout.fields[0].selection.offset, 3);
        }
        let declared_cycle = sample
            .replace("4, 0x0000,,,32,;", "4,0x0000,,, ,Assem201,8,Param2;")
            .replace(
                "\"Circular\",,,0,,, ,ASSEM101",
                "\"Circular\",,3,0,,, ,ASSEM201",
            );
        let profiles = super::super::parse(&declared_cycle).unwrap();
        let layout = profiles[0].input_layout.as_ref().unwrap();
        assert_eq!(layout.skipped_members, 1);
        assert_eq!(layout.fields[0].selection.offset, 3);
    }

    #[test]
    fn maps_boolean_members_across_byte_boundaries() {
        let source = include_str!("../../samples/loopback.eds")
            .replace(
                "[Assembly]",
                concat!(
                    "Param2 = 0,,,0,0xC1,,\"Ready\";\n",
                    "Param3 = 0,,,0,0xC1,1,\"Fault\";\n",
                    "Param4 = 0,,,0,0xC1,2,\"Invalid BOOL\";\n",
                    "Param5 = 0,,,0,0xC6,1,\"Counter\";\n",
                    "[Assembly]"
                ),
            )
            .replace(
                "4, 0x0000,,,32,;",
                ", 0x0000,,,1,Param2,6,,1,Param3,1,Param2,1,Param4,1,Param5,5,,,Param3,8,Param5;",
            );
        let profiles = super::super::parse(&source).unwrap();
        assert_eq!(profiles[0].settings.input_size, 4);
        let layout = profiles[0].input_layout.as_ref().unwrap();
        assert_eq!(layout.skipped_members, 2);
        let fields: Vec<_> = layout
            .fields
            .iter()
            .map(|field| {
                let selection = field.selection;
                (
                    field.name.as_str(),
                    selection.offset,
                    selection.bit,
                    selection.kind,
                )
            })
            .collect();
        assert_eq!(
            fields,
            [
                ("Ready", 0, Some(0), Kind::Bool),
                ("Fault", 0, Some(7), Kind::Bool),
                ("Ready_2", 1, Some(0), Kind::Bool),
                ("Fault_2", 2, None, Kind::Bool),
                ("Counter", 3, None, Kind::U8),
            ]
        );
        let bytes = [0x80, 0x01, 0x00, 42];
        let values: Vec<_> = layout
            .fields
            .iter()
            .map(|field| field.selection.decode(&bytes))
            .collect();
        assert_eq!(
            values,
            [
                Some(Value::Bool(false)),
                Some(Value::Bool(true)),
                Some(Value::Bool(true)),
                Some(Value::Bool(false)),
                Some(Value::Unsigned(42)),
            ]
        );
    }

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
