use crate::{
    capture::MAX_CAPTURE_SECONDS,
    eds,
    model::{ConnectionTimeout, Settings},
    settings_file::Preferences,
};
use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Serialize)]
pub struct Direction {
    pub assembly: u16,
    pub payload_bytes: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_payload_bytes: Option<u16>,
    pub requested_rpi_us: u32,
    pub run_idle_header: bool,
    pub priority: u8,
}

#[derive(Serialize)]
pub struct Origin {
    pub field: &'static str,
    pub state: Source,
    pub effective: String,
    pub eds: Option<String>,
    pub eds_size_is_layout: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Manual,
    Override,
    AssemblyDefinition,
    Eds,
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Override => "Override",
            Self::AssemblyDefinition => "Assembly definition",
            Self::Eds => "EDS",
        }
    }
}

#[derive(Serialize)]
pub struct Report {
    pub connection_valid: bool,
    pub capture_config_valid: bool,
    pub adapter_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub local_udp_port: u16,
    pub requests_output_ownership: bool,
    pub asserts_run: bool,
    pub configuration_assembly: Option<u16>,
    pub configuration_bytes: u16,
    pub large_forward_open: bool,
    pub electronic_key: Option<crate::model::ElectronicKey>,
    pub connection_timeout: ConnectionTimeout,
    pub connection_errors: Vec<String>,
    pub capture_errors: Vec<String>,
    pub eds_profile: Option<String>,
    pub input: Direction,
    pub output: Direction,
    pub origins: Vec<Origin>,
}

pub fn inspect(
    settings: &Settings,
    preferences: &Preferences,
    profile: Option<&eds::Profile>,
) -> Report {
    let mut connection_errors: Vec<_> = settings.validate().err().into_iter().collect();
    connection_errors.extend(preferences.integration.validate_for(settings).err());
    connection_errors.extend(crate::explicit::validate_saved_reads(&preferences.cip_reads).err());
    let mut capture_errors: Vec<_> = settings.csv_fields().err().into_iter().collect();
    if !(1..=1_000_000).contains(&preferences.capacity) {
        capture_errors.push("Capture capacity must be between 1 and 1,000,000 samples".into());
    }
    if preferences.gui_capture_seconds > MAX_CAPTURE_SECONDS {
        capture_errors.push(format!(
            "GUI capture duration must be 0–{MAX_CAPTURE_SECONDS} seconds"
        ));
    }
    if preferences.directory.trim().is_empty() {
        capture_errors.push("Enter a CSV output directory".into());
    }
    let mut origins = Vec::new();
    if let Some(profile) = profile {
        connection_errors.extend(profile.unsupported.iter().cloned());
        let imported = &profile.settings;
        let mut compare = |field, effective: String, from_eds: String| {
            let eds = profile.fields.get(field).cloned();
            let eds_size_is_layout = profile.size_from_assembly_definition(field);
            let state = if eds.is_none() {
                Source::Manual
            } else if effective != from_eds {
                Source::Override
            } else if eds_size_is_layout {
                Source::AssemblyDefinition
            } else {
                Source::Eds
            };
            origins.push(Origin {
                field,
                state,
                effective,
                eds,
                eds_size_is_layout,
            });
        };
        for (label, effective, from_eds) in [
            (
                "O->T assembly",
                settings.output_instance,
                imported.output_instance,
            ),
            (
                "T->O assembly",
                settings.input_instance,
                imported.input_instance,
            ),
            ("O->T size", settings.output_size, imported.output_size),
            ("T->O size", settings.input_size, imported.input_size),
            ("Config size", settings.config_size, imported.config_size),
        ] {
            compare(label, effective.to_string(), from_eds.to_string());
        }
        for (label, effective, from_eds) in [
            ("O->T RPI", settings.output_rpi_us, imported.output_rpi_us),
            ("T->O RPI", settings.input_rpi_us, imported.input_rpi_us),
        ] {
            compare(label, effective.to_string(), from_eds.to_string());
        }
        for (label, effective, from_eds) in [
            (
                "O->T priority",
                settings.output_priority,
                imported.output_priority,
            ),
            (
                "T->O priority",
                settings.input_priority,
                imported.input_priority,
            ),
        ] {
            compare(label, effective.to_string(), from_eds.to_string());
        }
        for (label, effective, from_eds) in [
            (
                "O->T header",
                settings.output_run_idle,
                imported.output_run_idle,
            ),
            (
                "T->O header",
                settings.input_run_idle,
                imported.input_run_idle,
            ),
            ("Ownership", settings.input_only, imported.input_only),
        ] {
            let describe = |value| match (label, value) {
                ("Ownership", true) => "Input-only",
                ("Ownership", false) => "Exclusive owner",
                (_, true) => "32-bit Run/Idle",
                (_, false) => "No Run/Idle",
            };
            compare(label, describe(effective).into(), describe(from_eds).into());
        }
        let config =
            |instance: Option<u16>| instance.map_or_else(|| "None".into(), |n| n.to_string());
        compare(
            "Config assembly",
            config(settings.config_instance),
            config(imported.config_instance),
        );
        let key = |key: Option<crate::model::ElectronicKey>| {
            key.map_or_else(
                || "None".into(),
                |key| {
                    format!(
                        "{}:{}:{} {}.{}",
                        key.vendor, key.device_type, key.product, key.major, key.minor
                    )
                },
            )
        };
        if settings.electronic_key.is_some() || imported.electronic_key.is_some() {
            compare(
                "Electronic key",
                key(settings.electronic_key),
                key(imported.electronic_key),
            );
        }
        if settings.large_forward_open || imported.large_forward_open {
            compare(
                "Large Forward_Open",
                settings.large_forward_open.to_string(),
                imported.large_forward_open.to_string(),
            );
        }
        if settings.input_min_size.is_some() || imported.input_min_size.is_some() {
            let mode = |min: Option<u16>| if min.is_some() { "Variable" } else { "Fixed" };
            compare(
                "T->O size mode",
                mode(settings.input_min_size).into(),
                mode(imported.input_min_size).into(),
            );
        }
        compare(
            "Timeout multiplier code",
            settings.timeout_multiplier_code.to_string(),
            imported.timeout_multiplier_code.to_string(),
        );
    }

    Report {
        connection_valid: connection_errors.is_empty(),
        capture_config_valid: connection_errors.is_empty() && capture_errors.is_empty(),
        adapter_ip: settings.device_ip,
        local_ip: settings.local_ip,
        local_udp_port: settings.local_udp_port,
        requests_output_ownership: !settings.input_only,
        asserts_run: settings.output_run_idle,
        configuration_assembly: settings.config_instance,
        configuration_bytes: settings.config_size,
        large_forward_open: settings.large_forward_open,
        electronic_key: settings.electronic_key,
        connection_timeout: settings.connection_timeout(None),
        connection_errors,
        capture_errors,
        eds_profile: profile.map(|profile| profile.id.clone()),
        input: Direction {
            assembly: settings.input_instance,
            payload_bytes: settings.input_size,
            minimum_payload_bytes: settings.input_min_size,
            requested_rpi_us: settings.input_rpi_us,
            run_idle_header: settings.input_run_idle,
            priority: settings.input_priority,
        },
        output: Direction {
            assembly: settings.output_instance,
            payload_bytes: settings.output_size,
            minimum_payload_bytes: None,
            requested_rpi_us: settings.output_rpi_us,
            run_idle_header: settings.output_run_idle,
            priority: settings.output_priority,
        },
        origins,
    }
}
