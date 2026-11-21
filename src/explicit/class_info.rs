use super::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes, le16};
use std::net::Ipv4Addr;

const OPTIONAL_INSTANCE_ATTRIBUTES: u32 = 4;
const OPTIONAL_SERVICES: u32 = 5;
pub(super) const ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Class revision"),
    (2, "Maximum instance ID"),
    (3, "Instance count"),
    (OPTIONAL_INSTANCE_ATTRIBUTES, "Optional instance attributes"),
    (OPTIONAL_SERVICES, "Optional services"),
    (6, "Maximum class attribute ID"),
    (7, "Maximum instance attribute ID"),
];

pub struct ClassInfo {
    pub class: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl ClassInfo {
    /// A usable inspection decoded at least one value and reached the end of the batch.
    /// Targets may reject unsupported common class attributes.
    pub fn is_usable(&self) -> bool {
        self.batch.is_usable_inspection()
    }

    /// IDs advertised by class attribute 4, when the target reports a valid list.
    pub fn optional_instance_attributes(&self) -> Option<Vec<u16>> {
        self.id_list(OPTIONAL_INSTANCE_ATTRIBUTES)
    }

    /// Service IDs advertised by class attribute 5, when the target reports a valid list.
    pub fn optional_services(&self) -> Option<Vec<u16>> {
        self.id_list(OPTIONAL_SERVICES)
    }

    fn id_list(&self, attribute: u32) -> Option<Vec<u16>> {
        self.batch
            .results
            .iter()
            .find_map(|entry| match &entry.outcome {
                SavedReadOutcome::Reply(bytes)
                    if entry.request.path.attribute == Some(attribute) =>
                {
                    decode_ids(bytes, "Class attribute ID list").ok()
                }
                _ => None,
            })
    }
}

/// Read common attributes of the selected class (instance zero) on one explicit session.
/// The batch retains the ordering, raw replies and CIP status of attributes 1 through 7.
pub fn read_class_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    class: u32,
    route: Option<Route>,
) -> Result<ClassInfo, String> {
    let batch = read_object_attributes(adapter, local, class, 0, route.as_ref(), ATTRIBUTES)?;
    Ok(ClassInfo {
        class,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Result<String, String> {
    if matches!(attribute, OPTIONAL_INSTANCE_ATTRIBUTES | OPTIONAL_SERVICES) {
        return decode_ids(bytes, "Class attribute ID list").map(|ids| format_ids(&ids));
    }
    if bytes.len() != 2 {
        return Err(format!(
            "Class attribute requires a two-byte UINT, got {} bytes",
            bytes.len()
        ));
    }
    Ok(le16(bytes, 0).to_string())
}

pub(super) fn decode_ids(bytes: &[u8], name: &str) -> Result<Vec<u16>, String> {
    let (count, values) = bytes
        .split_first_chunk()
        .ok_or_else(|| format!("{name} is missing its UINT count"))?;
    let count = usize::from(u16::from_le_bytes(*count));
    if values.len() != count * 2 {
        return Err(format!(
            "{name} declares {count} IDs, with {} bytes after its count",
            values.len()
        ));
    }
    Ok(values.chunks_exact(2).map(|word| le16(word, 0)).collect())
}

pub(super) fn format_ids(ids: &[u16]) -> String {
    let identifiers = ids
        .iter()
        .map(|id| format!("0x{id:04X}"))
        .collect::<Vec<_>>();
    format!("[{}]", identifiers.join(", "))
}
