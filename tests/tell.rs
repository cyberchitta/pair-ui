use pair_ui::{BadTell, MAX_TELL, TellRequest};
use proptest::prelude::*;
use serde_json::{Value, json};

#[test]
fn reads_the_project_line_and_the_json() {
    let tell = TellRequest::parse(b"sorted-studs\n{\"show\":[1,2]}").expect("a tell");
    assert_eq!(tell.project(), "sorted-studs");
    assert_eq!(tell.body(), &json!({ "show": [1, 2] }));
    assert_eq!(tell.into_body(), json!({ "show": [1, 2] }));
}

#[test]
fn splits_on_the_first_newline_only() {
    let tell = TellRequest::parse(b"p\n{\n\"a\": \"x\\ny\"\n}\n").expect("a tell");
    assert_eq!(tell.project(), "p");
    assert_eq!(tell.body(), &json!({ "a": "x\ny" }));
}

#[test]
fn refuses_a_request_with_no_project_line() {
    for bad in [&b""[..], b"{}", b"sorted-studs"] {
        assert!(
            matches!(TellRequest::parse(bad), Err(BadTell::NoProjectLine)),
            "{bad:?}"
        );
    }
}

#[test]
fn refuses_a_project_that_is_not_utf8() {
    assert!(matches!(
        TellRequest::parse(b"p\xff\n{}"),
        Err(BadTell::ProjectNotUtf8)
    ));
}

#[test]
fn refuses_a_body_that_is_not_json() {
    for bad in [&b"p\n"[..], b"p\nnot json", b"p\n{\"a\":", b"p\n{} {}"] {
        assert!(
            matches!(TellRequest::parse(bad), Err(BadTell::NotJson(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn takes_a_body_of_max_tell_and_refuses_one_byte_more() {
    let body = |len: usize| format!("\"{}\"", "a".repeat(len - 2));
    let at_limit = format!("p\n{}", body(MAX_TELL));
    assert!(TellRequest::parse(at_limit.as_bytes()).is_ok());
    let over = format!("p\n{}", body(MAX_TELL + 1));
    assert!(matches!(
        TellRequest::parse(over.as_bytes()),
        Err(BadTell::TooLarge(n)) if n == MAX_TELL + 1
    ));
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        ".{0,12}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::from),
            prop::collection::btree_map(".{0,6}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn a_tell_reads_back_as_the_client_writes_it(
        project in "[^\n]{0,24}",
        body in json_value(),
        pretty in any::<bool>(),
    ) {
        let json = if pretty {
            serde_json::to_string_pretty(&body).expect("serializes")
        } else {
            body.to_string()
        };
        let tell = TellRequest::parse(format!("{project}\n{json}").as_bytes())
            .expect("a tell");
        prop_assert_eq!(tell.project(), project.as_str());
        prop_assert_eq!(tell.body(), &body);
    }

    #[test]
    fn any_bytes_parse_or_fail_without_panicking(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let _ = TellRequest::parse(&bytes);
    }
}
