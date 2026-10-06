use std::path::Path;

use yashik::effective::build_effective;
use yashik::schema::Manifest;

#[test]
fn orca_tool_is_independent_from_harnesses_and_herdr() {
    let manifest: Manifest = serde_yaml::from_str(
        "version: 1\nharnesses:\n  codex: {}\ntools:\n  herdr: {}\n  orca: {}\n",
    )
    .unwrap();

    let effective = build_effective(&manifest, Path::new("/tmp/config")).unwrap();
    assert!(effective
        .harnesses
        .contains_key(&yashik::schema::HarnessId::Codex));
    assert_eq!(effective.herdr.unwrap().version, "latest");
    assert_eq!(effective.orca.unwrap().version, "latest");
}

#[test]
fn disabling_orca_does_not_disable_herdr_or_harnesses() {
    let manifest: Manifest = serde_yaml::from_str(
        "version: 1\nharnesses:\n  codex: {}\ntools:\n  herdr: {}\n  orca:\n    enabled: false\n",
    )
    .unwrap();

    let effective = build_effective(&manifest, Path::new("/tmp/config")).unwrap();
    assert!(effective
        .harnesses
        .contains_key(&yashik::schema::HarnessId::Codex));
    assert!(effective.herdr.is_some());
    assert!(effective.orca.is_none());
}
