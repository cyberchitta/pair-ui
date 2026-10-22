# pair-ui

Host one web page that a person and an agent drive together, on 127.0.0.1.

The person uses the page in a browser. The agent drives it through the app's
own commands, which hand the page opaque JSON (a tell) or ask it to show a
moniker (a reveal) through a Unix socket, and print its answer. The host never
reads what it carries: what the messages mean is between the app's commands
and its page, and the app's skill explains it.

The same messages also travel by `postMessage`, for a host that puts the page
in a frame. The page client in `page/` speaks both, so one build runs under
either host.

## Use it

**The page**, in any framework: depend on this repo pinned to a commit and
import the client.

```json
"dependencies": { "pair-ui": "github:cyberchitta/pair-ui#<commit>" }
```

```js
import { connect } from "pair-ui/page";

const host = await connect();           // null when opened as a plain file
show(host?.kept ?? defaultView);
host?.start({
  tell: (body) => ({ ok: true, message: "toolbar hidden" }),
  reveal: (moniker, state) => ({ ok: true, message: "part shown" }),
});
```

**The host and the commands**, from any language, with the binary:

```
cargo install --git https://github.com/cyberchitta/pair-ui
pair-ui serve <project> <build-dir> [--connect <origin>]... [--wasm]
pair-ui tell <project> '<json>'        # or - to read the JSON from stdin
pair-ui reveal <moniker> [--state '<json>']   # to the moniker's project
```

`serve` prints the address to open. `tell` and `reveal` print the page's
answer and exit 1 when it refused or no page answered. A Rust app can embed
the host and the client instead (`pair_ui::Host`, `pair_ui::client`), with
the build compiled in.

To try it with the demo page in this repo:

```
cargo run -- serve demo demo           # open the address it prints
cargo run -- tell demo '{"get":"state"}'
cargo run -- reveal demo://demo/clicks/7
```

The protocol is in the crate docs (`src/lib.rs`); building a page and its
commands on it is in `skills/pair-ui/SKILL.md`.
