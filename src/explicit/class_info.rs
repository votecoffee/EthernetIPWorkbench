use super::{ReadPath, Route, SavedRead, SavedReadBatch, le16, run_saved_reads};
use std::net::Ipv4Addr;

const ATTRIBUTES: [&str; 7] = [
    "Class revision",
    "Maximum instance ID",
    "Instance count",
    "Optional instance attributes",
    "Optional services",
    "Maximum class attribute ID",
    "Maximum instance attribute ID",
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
}

/// Read common attributes of the selected class (instance zero) on one explicit session.
/// The batch retains the ordering, raw replies and CIP status of attributes 1 through 7.
pub fn read_class_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    class: u32,
    route: Option<Route>,
) -> Result<ClassInfo, String> {
    let reads = ATTRIBUTES
        .iter()
        .enumerate()
        .map(|(index, name)| SavedRead {
            name: (*name).into(),
            path: ReadPath {
                class,
                instance: 0,
                attribute: Some(index as u32 + 1),
            },
            route: route.clone(),
            selection: None,
        })
        .collect();
    let batch = run_saved_reads(adapter, local, reads)?;
    Ok(ClassInfo {
        class,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Result<String, String> {
    if matches!(attribute, 4 | 5) {
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
