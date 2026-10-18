use super::*;
#[test]
fn package_registers_with_defaults() {
    let spec = crate::nodes::get("attribute_create").unwrap();
    assert_eq!(spec.name, NODE.name);
    Params::resolve(NODE.params, None).unwrap();
}
