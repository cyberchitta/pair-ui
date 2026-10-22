use pair_ui::Origin;

#[test]
fn tells_a_local_origin_from_a_remote_one() {
    for (text, local) in [
        ("http://localhost:3381", true),
        ("ws://localhost", true),
        ("http://selfcast.localhost:80", true),
        ("http://127.0.0.1:3381", true),
        ("http://127.8.9.10", true),
        ("http://[::1]:3381", true),
        ("https://api.example.com", false),
        ("https://localhost.example.com", false),
        ("wss://10.0.0.2:443", false),
        ("http://128.0.0.1", false),
        ("http://[fe80::1]", false),
    ] {
        let origin: Origin = text.parse().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(origin.is_local(), local, "{text}");
    }
}
