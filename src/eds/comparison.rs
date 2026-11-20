use super::{AssemblyLayout, DeviceIdentity, Profile, RpiRange};
use crate::model::{CaptureField, Selection};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct Change<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<T>,
}

impl<T: Clone + PartialEq> Change<T> {
    fn between(previous: Option<&T>, updated: Option<&T>) -> Option<Self> {
        (previous != updated).then(|| Self {
            previous: previous.cloned(),
            updated: updated.cloned(),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct AssemblyChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Change<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_members: Option<Change<usize>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Change<Selection>>,
}

#[derive(Debug, Serialize)]
pub struct RevisionComparison {
    pub connection_id: String,
    pub refresh_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Change<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Change<DeviceIdentity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_rpi_range: Option<Change<RpiRange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_rpi_range: Option<Change<RpiRange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<Change<Vec<String>>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub declarations: BTreeMap<String, Change<String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub assemblies: BTreeMap<&'static str, AssemblyChanges>,
}

impl Profile {
    /// Compare the imported data for the same connection in an updated EDS.
    /// Keep the differences available even when the replacement cannot be refreshed.
    pub fn compare_revision(&self, candidates: &[Self]) -> Result<RevisionComparison, String> {
        let updated = &candidates[self.matching_index(candidates)?];
        let refresh_blocker = self.check_refresh(updated).err();
        let assemblies = [
            ("input", &self.input_layout, &updated.input_layout),
            ("output", &self.output_layout, &updated.output_layout),
            (
                "configuration",
                &self.configuration_layout,
                &updated.configuration_layout,
            ),
        ]
        .into_iter()
        .filter_map(|(direction, previous, updated)| {
            let changes = assembly_changes(previous.as_ref(), updated.as_ref())?;
            Some((direction, changes))
        })
        .collect();
        Ok(RevisionComparison {
            connection_id: self.id.clone(),
            refresh_allowed: refresh_blocker.is_none(),
            refresh_blocker,
            name: Change::between(Some(&self.name), Some(&updated.name)),
            device: Change::between(self.device.as_ref(), updated.device.as_ref()),
            input_rpi_range: Change::between(
                self.input_rpi_range.as_ref(),
                updated.input_rpi_range.as_ref(),
            ),
            output_rpi_range: Change::between(
                self.output_rpi_range.as_ref(),
                updated.output_rpi_range.as_ref(),
            ),
            unsupported: Change::between(Some(&self.unsupported), Some(&updated.unsupported)),
            declarations: named_changes(
                self.fields
                    .iter()
                    .map(|(name, value)| (name.as_str(), value)),
                updated
                    .fields
                    .iter()
                    .map(|(name, value)| (name.as_str(), value)),
            ),
            assemblies,
        })
    }
}

fn assembly_changes(
    previous: Option<&AssemblyLayout>,
    updated: Option<&AssemblyLayout>,
) -> Option<AssemblyChanges> {
    let format = Change::between(previous.map(|p| &p.format), updated.map(|p| &p.format));
    let skipped_members = Change::between(
        previous.map(|p| &p.skipped_members),
        updated.map(|p| &p.skipped_members),
    );
    let fields = named_changes(
        previous
            .into_iter()
            .flat_map(|p| p.fields.iter())
            .map(named_selection),
        updated
            .into_iter()
            .flat_map(|p| p.fields.iter())
            .map(named_selection),
    );
    (format.is_some() || skipped_members.is_some() || !fields.is_empty()).then_some(
        AssemblyChanges {
            format,
            skipped_members,
            fields,
        },
    )
}

fn named_selection(field: &CaptureField) -> (&str, &Selection) {
    (&field.name, &field.selection)
}

fn named_changes<'a, T: Clone + PartialEq + 'a>(
    previous: impl Iterator<Item = (&'a str, &'a T)>,
    updated: impl Iterator<Item = (&'a str, &'a T)>,
) -> BTreeMap<String, Change<T>> {
    let mut values = BTreeMap::<_, (Option<&T>, Option<&T>)>::new();
    for (name, value) in previous {
        values.entry(name).or_default().0 = Some(value);
    }
    for (name, value) in updated {
        values.entry(name).or_default().1 = Some(value);
    }
    values
        .into_iter()
        .filter_map(|(name, (previous, updated))| {
            Some((name.to_owned(), Change::between(previous, updated)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    #[test]
    fn compares_typed_fields_by_name_and_reports_each_changed_selection() {
        let mut prior = super::super::parse(include_str!("../../samples/loopback.eds"))
            .unwrap()
            .remove(0);
        let field = |name: &str, offset| CaptureField {
            name: name.to_owned(),
            selection: Selection {
                offset,
                kind: Kind::U8,
                ..Selection::default()
            },
        };
        prior.input_layout = Some(AssemblyLayout {
            format: "Assem101".into(),
            skipped_members: 0,
            fields: vec![field("Weight", 0), field("Old status", 2), field("Mode", 3)],
        });
        let unchanged = prior
            .compare_revision(std::slice::from_ref(&prior))
            .unwrap();
        assert!(unchanged.refresh_allowed);
        assert!(unchanged.declarations.is_empty() && unchanged.assemblies.is_empty());

        let mut updated = prior.clone();
        updated.input_layout.as_mut().unwrap().fields =
            vec![field("Mode", 3), field("Weight", 2), field("New status", 0)];
        let report = prior.compare_revision(&[updated]).unwrap();
        assert!(report.refresh_allowed);
        let changed = &report.assemblies["input"].fields;
        assert_eq!(changed.len(), 3);
        assert_eq!(changed["Weight"].previous.unwrap().offset, 0);
        assert_eq!(changed["Weight"].updated.unwrap().offset, 2);
        assert_eq!(changed["Old status"].updated, None);
        assert_eq!(changed["New status"].previous, None);
        let serialized = toml::to_string_pretty(&report).unwrap();
        let parsed: toml::Table = toml::from_str(&serialized).unwrap();
        let fields = &parsed["assemblies"]["input"]["fields"];
        assert_eq!(fields["Weight"]["updated"]["offset"].as_integer(), Some(2));
        assert!(fields["Old status"].get("updated").is_none());
        assert!(fields["New status"].get("previous").is_none());
    }
}
