//! Attribute names shared by the browser and fixed object inspections.

use super::{
    ASSEMBLY_CLASS, CONNECTION_MANAGER_CLASS, ETHERNET_LINK_CLASS, LLDP_DATA_TABLE_CLASS,
    PORT_OBJECT_CLASS, QOS_CLASS, TCPIP_INTERFACE_CLASS, assembly_info, class_info, link_info,
    lldp_neighbor, manager_info, object_list, port_info, qos_info, tcpip_info,
};

const IDENTITY_ATTRIBUTES: &[(u32, &str)] = &[
    (1, "Vendor ID"),
    (2, "Device type"),
    (3, "Product code"),
    (4, "Revision"),
    (5, "Status"),
    (6, "Serial number"),
    (7, "Product name"),
];

/// Common attribute choices for a CIP object. Device support is determined by its reply.
pub fn known_attributes(class: u32, instance: u32) -> &'static [(u32, &'static str)] {
    match (class, instance) {
        (_, 0) => class_info::ATTRIBUTES,
        (1, _) => IDENTITY_ATTRIBUTES,
        (object_list::MESSAGE_ROUTER_CLASS, _) => object_list::ATTRIBUTES,
        (ASSEMBLY_CLASS, _) => assembly_info::ATTRIBUTES,
        (CONNECTION_MANAGER_CLASS, _) => manager_info::COUNTERS,
        (QOS_CLASS, _) => qos_info::ATTRIBUTES,
        (PORT_OBJECT_CLASS, _) => port_info::INSTANCE,
        (TCPIP_INTERFACE_CLASS, _) => tcpip_info::ATTRIBUTES,
        (ETHERNET_LINK_CLASS, _) => link_info::ATTRIBUTES,
        (LLDP_DATA_TABLE_CLASS, _) => lldp_neighbor::ATTRIBUTES,
        _ => &[],
    }
}
