use pair_ui::{BadMoniker, Moniker};
use proptest::prelude::*;

fn parse(text: &str) -> Moniker {
    text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
}

#[test]
fn parses_monikers() {
    let m = parse("khana://sorted-studs/joint_demo#part=stacked_upper_column");
    assert_eq!(m.scheme(), "khana");
    assert_eq!(m.project(), "sorted-studs");
    assert_eq!(
        m.to_string(),
        "khana://sorted-studs/joint_demo#part=stacked_upper_column"
    );
    assert_eq!(parse("seed-round://seed-round").project(), "seed-round");
    assert_eq!(parse("a.b+c://p?x=1").project(), "p");
}

#[test]
fn refuses_malformed_monikers() {
    for bad in [
        "khana://",
        "khana:///x",
        "khana://../x",
        "khana://p/a b",
        "khana://p\tx",
    ] {
        assert!(
            matches!(bad.parse::<Moniker>(), Err(BadMoniker::Malformed(_))),
            "{bad}"
        );
    }
}

#[test]
fn tells_a_non_moniker_from_a_malformed_one() {
    for other in [
        "/a/path",
        "src/main.rs:12",
        "Khana://p",
        "1x://p",
        " khana://p",
        "://p",
        "",
    ] {
        assert!(
            matches!(other.parse::<Moniker>(), Err(BadMoniker::NotAMoniker(_))),
            "{other:?}"
        );
    }
}

proptest! {
    #[test]
    fn a_moniker_prints_as_it_parses(
        scheme in "[a-z][a-z0-9+.-]{0,8}",
        project in "[A-Za-z0-9._-]{1,12}",
        rest in "([/?#][!-~]{0,20})?",
    ) {
        prop_assume!(project != "." && project != "..");
        let text = format!("{scheme}://{project}{rest}");
        let m = parse(&text);
        prop_assert_eq!(m.to_string(), text);
        prop_assert_eq!(m.project(), project.as_str());
    }
}
