//! Read-only comparison of profile payload lengths with adapter assembly sizes.

use super::{
    ASSEMBLY_CLASS, ReadPath, Route, SavedRead, SavedReadBatch, SavedReadOutcome,
    assembly_info::{SIZE_ATTRIBUTE, size},
    run_saved_reads,
};
use crate::model::Settings;
use serde::Serialize;

pub struct ProfileAssemblyCheck {
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
    pub checks: Vec<AssemblySizeCheck>,
}

#[derive(Serialize)]
pub struct AssemblySizeCheck {
    pub role: &'static str,
    pub instance: u16,
    pub minimum_bytes: usize,
    pub maximum_bytes: usize,
    pub declared_size_bytes: Option<u16>,
    pub matches_profile: Option<bool>,
}

impl ProfileAssemblyCheck {
    /// True when every configured assembly role has a declared size within its range.
    pub fn matches_profile(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.matches_profile == Some(true))
    }
}

/// Compare the profile's application byte lengths with the target's size attributes.
/// The check uses one registered session and reads each configured instance once.
pub fn check_profile_assemblies(settings: &Settings) -> Result<ProfileAssemblyCheck, String> {
    let route = settings.parsed_io_route()?;
    let mut assemblies = vec![
        ("Input", settings.input_instance, settings.input_lengths()?),
        (
            "Output",
            settings.output_instance,
            settings.output_lengths()?,
        ),
    ];
    if settings.config_size > 0 {
        let instance = settings
            .config_instance
            .ok_or("Configuration bytes require an assembly instance")?;
        let bytes = usize::from(settings.config_size);
        assemblies.push(("Configuration", instance, bytes..=bytes));
    }

    let mut reads: Vec<SavedRead> = Vec::with_capacity(assemblies.len());
    for &(role, instance, _) in &assemblies {
        if instance == 0 {
            return Err(format!("{role} assembly instance must be nonzero"));
        }
        if !reads
            .iter()
            .any(|read| read.path.instance == u32::from(instance))
        {
            reads.push(SavedRead {
                name: format!("Assembly {instance} declared size"),
                path: ReadPath {
                    class: ASSEMBLY_CLASS,
                    instance: u32::from(instance),
                    attribute: Some(SIZE_ATTRIBUTE),
                },
                route: route.clone(),
                selection: None,
            });
        }
    }

    let batch = run_saved_reads(settings.device_ip, settings.local_ip, reads)?;
    let checks = assemblies
        .into_iter()
        .map(|(role, instance, range)| {
            let declared = batch.results.iter().find_map(|entry| match &entry.outcome {
                SavedReadOutcome::Reply(bytes)
                    if entry.request.path.instance == u32::from(instance) =>
                {
                    size(bytes).ok()
                }
                _ => None,
            });
            AssemblySizeCheck {
                role,
                instance,
                minimum_bytes: *range.start(),
                maximum_bytes: *range.end(),
                declared_size_bytes: declared,
                matches_profile: declared.map(|size| range.contains(&usize::from(size))),
            }
        })
        .collect();
    Ok(ProfileAssemblyCheck {
        route,
        batch,
        checks,
    })
}
