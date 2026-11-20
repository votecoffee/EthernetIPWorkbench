use super::{USAGE, explicit_endpoint, parse_expected_identity};
use assembly_scanner::{
    explicit::{AttributeWrite, IdentityPin, ReadPath, Route},
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
    let mut readback = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        if option == "--route" && route.is_none() {
            let value = options.next().ok_or(USAGE)?;
            route = Some(value.to_string_lossy().parse()?);
        } else if option == "--apply" && apply.is_none() {
            let value = options.next().ok_or(USAGE)?;
            let expected = parse_expected_identity(&value.to_string_lossy())?;
            let serial = expected
                .serial
                .ok_or("Apply requires decimal vendor:type:product:serial")?;
            apply = Some(IdentityPin {
                key: expected.key,
                serial,
            });
        } else if option == "--readback" && !readback {
            readback = true;
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
    if readback {
        report["readback_requested"] = json!(true);
    }
    let success = if let Some(pin) = apply {
        let result = if readback {
            request.apply_with_readback(pin)?
        } else {
            request.apply(pin)?
        };
        let success = result.is_success();
        report["result"] = json!(result);
        success
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
