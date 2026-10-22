---
name: pair-ui
description: Build a web page that a person and an agent drive together, and the app commands that drive it, on pair-ui's protocol. Covers monikers, the page client in pair-ui's `page/` (connect, start, keep), answering tells and reveals, what a page must record and refuse, the commands' side (`pair_ui::client::tell` and `reveal`), and serving a build with `pair_ui::Host` under its page policy. Load before writing or changing such a page, its commands, or the skill that teaches an agent to use them.
---

# pair-ui

One web page, used by a person and driven by an agent. The person uses the
page; the agent runs the app's own commands, which hand the page a message
and print its answer. The host carries messages and never reads them: what
they mean is between the app's commands and its page, and the app's skill
explains it.

A page runs under one of two kinds of host, with the same messages:

- **The pair-ui host** (`pair_ui::Host`), in a plain browser: the page talks
  HTTP to its own origin, the commands talk to a Unix socket.
- **A framing host**, which puts the page in a frame: messages go by
  `postMessage`. Such a host serves each project's page from an origin whose
  host is the project's name, and opens no links from it.

The page client in `page/` speaks both, so one build runs under either. The
wire protocol is in the crate docs (`src/lib.rs`).

## Monikers: name what the page shows

```
<scheme>://<project>/<rest>      selfcast://selfcast/thread/37:0xcd34…#cast=1:0x2
```

- **The scheme names the app**: lowercase, `[a-z][a-z0-9+.-]*`. One app in
  many projects uses one scheme. The pair-ui host serves one page and does
  not read it; a host that runs many apps routes by it.
- **The project** is the one the host serves; the page reads it as
  `host.project`. The pair-ui host refuses a reveal naming another one.
- **`<rest>` is the app's.** It opens with `/`, `?` or `#`, or is empty.
- **One token:** no whitespace or control characters, so it pastes whole
  into a chat or a commit message.
- **It resolves without the page**, through the app's own command. The
  app's skill says what the form means.
- **Prefer names to positions.** An index resolves confidently to the wrong
  thing after an edit. A moniker that no longer resolves says so.

## The page

Bundle the client (plain JavaScript, typed by `page/index.d.ts`). Add the
repo as a dependency, pinned to a commit
(`"pair-ui": "github:cyberchitta/pair-ui#<sha>"`), and import `pair-ui/page`:

```js
import { connect } from "pair-ui/page";

const host = await connect();          // null when opened as a plain file
// host.project, host.opensLinks, host.kept
show(host?.kept ?? defaultView);
host?.start({
  reveal: (moniker, state) => ({ ok: true, message: "part X, camera put back" }),
  tell: (body) => ({ ok: true, message: "toolbar hidden", result: { … } }),
});
host?.keep(view);                       // JSON under 64 KB; null forgets it
```

- **`start` once, after the kept view is drawn.** It tells the host the page
  can answer; a framing host holds reveals until then.
- **Answer within 10 s.** A handler may return a promise. One that throws
  refuses with its error's message.
- **Handlers can overlap.** The next message arrives while an earlier one
  is still being answered, so a slow reveal does not hold up a tell. Two
  reveals can be in flight at once: let the later one win; the overtaken
  one changes nothing and says so.
- **A message the page cannot do in full is refused and changes nothing.**
  The agent cannot see the page, so a half-applied message reads as done.
- **A reveal reconstructs a view from a moniker.** What it puts back is the
  app's choice; say what in `message`. Leave out `reveal` and every reveal
  is refused.
- **`result` is how a command asks the page something** (what it shows,
  what the person picked). The host passes it through unread.
- **What the person picked is the page's own state,** read by a tell's
  `result`. Record what the person did, never what a tell or a reveal did:
  an agent asking what the person chose must not get back what an agent
  chose.
- **Keep one value**, replaced on each `keep`. A page needing several views
  keys them inside it. With no kept value, draw the default view.
- **Check `host.opensLinks`** before making a link open a tab; where it is
  false, show the address instead.

## The commands

From Rust, through the project's socket (`pair_ui::socket_path(project)`):

```rust
let answer = pair_ui::client::tell(&socket, project, &body).await?;
let moniker: pair_ui::Moniker = text.parse()?; // its socket: socket_path(moniker.project())
let answer = pair_ui::client::reveal(&socket, &moniker, Some(&state)).await?;
// Answer { ok, message, result }: ok == false is the page's refusal
```

- **Pass a refusal on and exit non-zero.** An `Err` means no page answered:
  say whether the page is open.
- **Put a version in the bodies** (`{ "selfcast": 1, … }`) and have the page
  refuse one it does not take: the command is installed and the page was
  built, so the two can drift.
- **No agent writes a body.** The agent runs the app's command and reads
  what it prints.

## Serving a build

```rust
let host = pair_ui::Host::bind(pair_ui::Config {
    project,
    build: pair_ui::Build::Folder(dist), // or Build::Embedded(&[("index.html", include_bytes!(…)), …])
    policy: pair_ui::Policy { connect: vec!["http://localhost:3381".parse()?], wasm: true },
    socket: pair_ui::socket_path(&project),
}).await?;
println!("{}", host.page_url());  // carries the token: show it to the person, not to logs
host.serve().await?;
```

The page is served under a Content-Security-Policy, so build for it:

- **No inline scripts**: load the script from a file beside `index.html`.
  Inline styles are allowed.
- **No network beyond the page's own origin** except `connect` origins, and
  those only for fetches and sockets: images and scripts load only from the
  build. Adding a remote origin sends data off the machine; ask first.
- **`wasm: true`** lets the page compile WebAssembly.

An app not written in Rust runs the host with the `pair-ui` binary instead
(`cargo install --git https://github.com/cyberchitta/pair-ui`): `pair-ui
serve <project> <build-dir> [--connect <origin>]... [--wasm]`, and its
commands call `pair-ui tell <project> '<json>'` (`-` reads stdin) and
`pair-ui reveal <moniker> [--state '<json>']`, which reaches the host of
the moniker's project. Both print the
page's answer as `told …`/`revealed …` then `message:` and `result:` lines,
and exit 1 when the page refused or none answered.

Try it with the demo: `cargo run -- serve demo demo`, open the address, then
`cargo run -- tell demo '{"get":"state"}'` and
`cargo run -- reveal demo://demo/clicks/7`.

## Ship a skill with the app

An agent drives the page through the app's skill, which needs both:

- **Text** saying when to drive the page, what each command does, and what
  it cannot know (whether the person is looking, who chose what is shown).
- **The commands** that do it.
