use super::{USAGE, explicit_endpoint, parse_expected_identity};
use assembly_scanner::{
    explicit::{AttributeWrite, Class3Size, IdentityPin, ReadPath, Route, WriteOptions},
    model::{ByteSwap, Kind, Selection, hex, parse_hex},
};
use serde_json::json;
use std::ffi::{OsStr, OsString};

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
                let value = options.next().ok_or(USAGE)?;
                let expected = parse_expected_identity(&value.to_string_lossy())?;
                let serial = expected
                    .serial
                    .ok_or("Apply requires decimal vendor:type:product:serial")?;
                apply = Some(IdentityPin {
                    key: expected.key,
                    serial,
                });
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
    let request = AttributeWrite {
        adapter_or_bridge,
        local_ip,
        path,
        route,
        value: bytes,
    };
    let mut report = json!({
        "adapter_or_bridge": request.adapter_or_bridge,
        "local_ip": request.local_ip,
        "route": request.route,
        "path": request.path,
        "service": "Set_Attribute_Single",
        "value_hex": hex(&request.value),
        "bytes": request.value.len(),
    });
    if let Some(format) = format {
        report["encoding"] = json!(format);
        report["typed_value"] = json!(value);
    }
    if readback {
        report["readback_requested"] = json!(true);
    }
    if let Some(bytes) = &expected_current {
        report["expected_current_hex"] = json!(hex(bytes));
    }
    let class3 = connected.then(|| connection_size.unwrap_or_default());
    if let Some(size) = class3 {
        report["class3_size"] = json!(size);
        report["large_forward_open"] = json!(size.uses_large_forward_open());
    }
    let success = if let Some(pin) = apply {
        let options = WriteOptions {
            expected_current: expected_current.as_deref(),
            readback,
        };
        let result = match class3 {
            Some(size) => request.apply_connected(pin, options, size),
            None => request.apply_with_options(pin, options),
        }?;
        let success = result.is_success();
        report["result"] = json!(result);
        success
    } else {
        match class3 {
            Some(size) => request.validate_connected(size)?,
            None => request.validate()?,
        }
        report["result"] = json!({"outcome": "preview"});
        true
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(success)
}
