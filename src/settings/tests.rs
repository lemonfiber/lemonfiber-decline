use std::path::PathBuf;

use super::Settings;

#[test]
fn the_stack_defaults_hold_where_nothing_is_set() {
    let settings = Settings::from(|_| None);

    assert_eq!(settings.config, PathBuf::from("/config"));
    assert_eq!(settings.jellyfin, "http://jellyfin:8096");
}

#[test]
fn a_set_variable_takes_the_place_of_its_default() {
    let settings = Settings::from(|name| match name {
        "LEMONFIBER_DECLINE_CONFIG" => Some("/elsewhere".to_owned()),
        "LEMONFIBER_DECLINE_JELLYFIN" => Some("http://media:8096/".to_owned()),
        _ => None,
    });

    assert_eq!(settings.config, PathBuf::from("/elsewhere"));
    assert_eq!(settings.jellyfin, "http://media:8096");
}
