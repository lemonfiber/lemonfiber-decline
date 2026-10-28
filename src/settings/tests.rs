use std::path::PathBuf;

use super::Settings;

#[test]
fn the_stack_defaults_hold_where_nothing_is_set() {
    let settings = Settings::from(&|_| None);

    assert_eq!(
        settings.as_ref().map(|settings| settings.config.clone()),
        Ok(PathBuf::from("/config"))
    );
    assert_eq!(
        settings.map(|settings| settings.jellyfin.to_string()),
        Ok("https://jellyfin:8920/".to_owned())
    );
}

#[test]
fn a_set_variable_takes_the_place_of_its_default() {
    let settings = Settings::from(&|name| match name {
        "LEMONFIBER_DECLINE_CONFIG" => Some("/elsewhere".to_owned()),
        "LEMONFIBER_DECLINE_JELLYFIN" => Some("https://media:8920/".to_owned()),
        _ => None,
    });

    assert_eq!(
        settings.as_ref().map(|settings| settings.config.clone()),
        Ok(PathBuf::from("/elsewhere"))
    );
    assert_eq!(
        settings.map(|settings| settings.jellyfin.to_string()),
        Ok("https://media:8920/".to_owned())
    );
}

#[test]
fn a_media_server_at_anything_but_an_https_address_is_refused() {
    for named in [
        "http://jellyfin:8096",
        "jellyfin:8920",
        "",
        "https://user:secret@jellyfin:8920",
    ] {
        let settings = Settings::from(&|name| {
            (name == "LEMONFIBER_DECLINE_JELLYFIN").then(|| named.to_owned())
        });

        assert!(
            settings.is_err_and(|why| why.contains("HTTPS address alone")),
            "{named}"
        );
    }
}
