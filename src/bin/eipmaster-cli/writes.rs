use super::{USAGE, explicit_endpoint, parse_expected_identity};
use assembly_scanner::{
    explicit::{AttributeWrite, Class3Size, IdentityPin, ReadPath, Route, WriteDraft},
    model::{ByteSwap, Kind, Selection, parse_hex},
};
use serde_json::json;
use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

enum Action<'a> {
    Preview,
    Save(&'a Path),
    Verify,
    Apply(IdentityPin),
}

fn identity_pin(value: &OsStr) -> Result<IdentityPin, String> {
    let expected = parse_expected_identity(&value.to_string_lossy())?;
    let serial = expected
        .serial
        .ok_or("Apply requires decimal vendor:type:product:serial")?;
    Ok(IdentityPin {
        key: expected.key,
        serial,
    })
}

pub(super) fn file(path: &Path, options: &[OsString]) -> Result<bool, String> {
    let action = match options {
        [] => Action::Preview,
        [flag] if flag == "--verify" => Action::Verify,
        [flag, value] if flag == "--apply" => Action::Apply(identity_pin(value)?),
        _ => return Err(USAGE.into()),
    };
    run(&WriteDraft::load(path)?, action, None)
}

pub(super) fn attribute(
    adapter: &OsStr,
    local: &OsStr,
    path: ReadPath,
    value: &OsStr,
    options: &[OsString],
) -> Result<bool, String> {
    let (adapter_or_bridge, local_ip, _) = explicit_endpoint(adapter, local, &[])?;
    let mut route: Option<Route> = None;
    let mut apply = None;
    let mut save = None;
    let mut readback = false;
    let mut connected = false;
    let mut connection_size = None;
    let mut expected_current = None;
    let mut kind = None;
    let mut endian = None;
    let mut swap = None;
    let mut text_bytes = None;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.to_str() {
            Some("--route") if route.is_none() => {
                route = Some(options.next().ok_or(USAGE)?.to_string_lossy().parse()?);
            }
            Some("--apply") if apply.is_none() => {
                apply = Some(identity_pin(options.next().ok_or(USAGE)?)?);
            }
            Some("--save-draft") if save.is_none() => {
                save = Some(Path::new(options.next().ok_or(USAGE)?));
            }
            Some("--readback") if !readback => readback = true,
            Some("--connected") if !connected => connected = true,
            Some("--connection-size") if connection_size.is_none() => {
                let bytes = options
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u16>().ok())
                    .ok_or("Class 3 connection size must be an integer")?;
                connection_size = Some(Class3Size::try_from(bytes)?);
            }
            Some("--expect-current") if expected_current.is_none() => {
                let value = options.next().ok_or(USAGE)?.to_string_lossy();
                expected_current = Some(
                    parse_hex(&value)
                        .map_err(|error| format!("Expected current bytes · {error}"))?,
                );
            }
            Some("--type") if kind.is_none() => {
                let name = options.next().ok_or(USAGE)?.to_string_lossy();
                kind = Some(
                    Kind::ALL
                        .into_iter()
                        .find(|kind| format!("{kind:?}").eq_ignore_ascii_case(&name))
                        .ok_or_else(|| format!("Unknown typed value kind {name:?}"))?,
                );
            }
            Some("--endian") if endian.is_none() => {
                endian = Some(match options.next().and_then(|value| value.to_str()) {
                    Some("little") => false,
                    Some("big") => true,
                    _ => return Err("Byte order must be little or big".into()),
                });
            }
            Some("--swap") if swap.is_none() => {
                swap = Some(match options.next().and_then(|value| value.to_str()) {
                    Some("none") => ByteSwap::None,
                    Some("bytes_in_words") => ByteSwap::BytesInWords,
                    Some("word_order") => ByteSwap::WordOrder,
                    _ => return Err("Swap must be none, bytes_in_words, or word_order".into()),
                });
            }
            Some("--text-bytes") if text_bytes.is_none() => {
                text_bytes = Some(
                    options
                        .next()
                        .ok_or(USAGE)?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Text byte length must be an integer")?,
                );
            }
            _ => return Err(USAGE.into()),
        }
    }
    if kind.is_none() && (endian.is_some() || swap.is_some() || text_bytes.is_some()) {
        return Err("Byte order, swap, and text length options require --type".into());
    }
    if !connected && connection_size.is_some() {
        return Err("--connection-size requires --connected".into());
    }
    let action = match (apply, save) {
        (Some(_), Some(_)) => {
            return Err("--save-draft and --apply require separate commands".into());
        }
        (Some(pin), None) => Action::Apply(pin),
        (None, Some(path)) => Action::Save(path),
        (None, None) => Action::Preview,
    };
    let format = kind.map(|kind| Selection {
        kind,
        big_endian: endian.unwrap_or(false),
        swap: swap.unwrap_or_default(),
        string_length: text_bytes,
        ..Selection::default()
    });
    let value = value.to_string_lossy();
    let bytes = match format {
        Some(format) => format.encode_value(&value)?,
        None => parse_hex(&value)?,
    };
    let draft = WriteDraft {
        request: AttributeWrite {
            adapter_or_bridge,
            local_ip,
            path,
            route,
            value: bytes,
        },
        class3_size: connected.then(|| connection_size.unwrap_or_default()),
        expected_current,
        readback,
    };
    run(
        &draft,
        action,
        format.map(|format| (format, value.as_ref())),
    )
}

fn run(
    draft: &WriteDraft,
    action: Action<'_>,
    typed: Option<(Selection, &str)>,
) -> Result<bool, String> {
    let mut report = draft.report_document();
    if let Some((format, value)) = typed {
        report["encoding"] = json!(format);
        report["typed_value"] = json!(value);
    }
    let (success, result) = match action {
        Action::Preview => {
            draft.validate()?;
            (true, json!({"outcome": "preview"}))
        }
        Action::Save(path) => {
            draft.save_new(path)?;
            report["draft_saved"] = json!(path);
            (true, json!({"outcome": "preview"}))
        }
        Action::Verify => {
            let identity = draft.read_target_identity()?;
            (
                true,
                json!({"outcome": "identity_read", "observed_identity": identity}),
            )
        }
        Action::Apply(pin) => {
            let result = draft.apply(pin)?;
            (result.is_success(), json!(result))
        }
    };
    report["result"] = result;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(success)
}
