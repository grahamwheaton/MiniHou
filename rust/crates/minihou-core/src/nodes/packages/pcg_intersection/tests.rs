use super::*;
#[test]
fn spec_resolves_defaults() {
    assert!(Params::resolve(NODE.params, None).is_ok());
}
