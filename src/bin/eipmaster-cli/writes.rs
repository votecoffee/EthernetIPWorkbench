use super::{USAGE, explicit_endpoint, parse_expected_identity};
use assembly_scanner::{
    explicit::{AttributeWrite, IdentityPin, ReadPath, Route, WriteOutcome},
    model::{hex, parse_hex},
};
use serde_json::json;
use std::ffi::{OsStr, OsString};

pub(super) fn attribute(
    adapter: &OsStr,
    local: &OsStr,
    path: ReadPath,
    bytes: &OsStr,
    options: &[OsString],
) -> Result<bool, String> {
    let (adapter_or_bridge, local_ip, _) = explicit_endpoint(adapter, local, &[])?;
    let mut route: Option<Route> = None;
    let mut apply = None;
    let options = options.chunks_exact(2);
    if !options.remainder().is_empty() {
        return Err(USAGE.into());
    }
    for pair in options {
        if pair[0] == "--route" && route.is_none() {
            route = Some(pair[1].to_string_lossy().parse()?);
        } else if pair[0] == "--apply" && apply.is_none() {
            let expected = parse_expected_identity(&pair[1].to_string_lossy())?;
            let serial = expected
                .serial
                .ok_or("Apply requires decimal vendor:type:product:serial")?;
            apply = Some(IdentityPin {
                key: expected.key,
                serial,
            });
        } else {
            return Err(USAGE.into());
        }
    }
    let request = AttributeWrite {
        adapter_or_bridge,
        local_ip,
        path,
        route,
        value: parse_hex(&bytes.to_string_lossy())?,
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
    let success = if let Some(pin) = apply {
        let result = request.apply(pin)?;
        let acknowledged = matches!(result.outcome, WriteOutcome::Acknowledged { .. });
        report["result"] = json!(result);
        acknowledged
    } else {
        request.validate()?;
        report["result"] = json!({"outcome": "preview"});
        true
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(success)
}
