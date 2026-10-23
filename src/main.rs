//! `pair-ui`: run a host for a built page, and hand that page tells and
//! reveals, for apps that do not embed the library.
//!
//! The output matches the `shala` CLI's, so an app's skill reads the same
//! under either host. Exit 0 when the page answered `ok`, 1 when it refused
//! or no page answered, 2 for a usage error.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use pair_ui::{
    Answer, BadMoniker, Build, Config, Host, Moniker, Origin, Policy, client, socket_path,
};
use serde_json::Value;

const USAGE: &str = "\
usage: pair-ui serve <project> <build-dir> [--connect <origin>]... [--wasm] [--images]
       pair-ui tell <project> '<json>'      (or - to read the JSON from stdin)
       pair-ui reveal <moniker> [--state '<json>']";

#[derive(Debug, PartialEq)]
enum Command {
    Serve {
        project: String,
        build: PathBuf,
        connect: Vec<Origin>,
        wasm: bool,
        remote_images: bool,
    },
    Tell {
        project: String,
        body: Body,
    },
    Reveal {
        moniker: Moniker,
        state: Option<Value>,
    },
}

/// A tell's JSON, given inline or to be read from stdin.
#[derive(Debug, PartialEq)]
enum Body {
    Given(Value),
    Stdin,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(why) => {
            eprintln!("pair-ui: {why}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("pair-ui: {why}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [cmd, project, build, flags @ ..] if cmd == "serve" => serve_args(project, build, flags),
        [cmd, project, body] if cmd == "tell" => Ok(Command::Tell {
            project: project.clone(),
            body: body_arg(body)?,
        }),
        [cmd, rest @ ..] if cmd == "reveal" => reveal_args(rest),
        _ => Err("expected serve, tell or reveal, with their arguments".into()),
    }
}

fn serve_args(project: &str, build: &str, flags: &[String]) -> Result<Command, String> {
    let (mut connect, mut flags) = (Vec::new(), flags.iter());
    let (mut wasm, mut remote_images) = (false, false);
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--wasm" => wasm = true,
            "--images" => remote_images = true,
            "--connect" => {
                let origin = flags.next().ok_or("--connect needs an origin")?;
                connect.push(origin.parse().map_err(|e| format!("{e}"))?);
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok(Command::Serve {
        project: project.into(),
        build: build.into(),
        connect,
        wasm,
        remote_images,
    })
}

fn body_arg(body: &str) -> Result<Body, String> {
    match body {
        "-" => Ok(Body::Stdin),
        json => json_arg(json).map(Body::Given),
    }
}

fn reveal_args(rest: &[String]) -> Result<Command, String> {
    let (moniker, state) = match rest {
        [moniker] => (moniker, None),
        [moniker, flag, state] | [flag, state, moniker] if flag == "--state" => {
            (moniker, Some(json_arg(state)?))
        }
        _ => return Err("reveal takes a moniker and an optional --state".into()),
    };
    Ok(Command::Reveal {
        moniker: moniker.parse().map_err(|e: BadMoniker| e.to_string())?,
        state,
    })
}

fn json_arg(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("not JSON ({e}): {text}"))
}

async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Serve {
            project,
            build,
            connect,
            wasm,
            remote_images,
        } => {
            let policy = Policy {
                connect,
                wasm,
                remote_images,
            };
            serve(project, build, policy).await
        }
        Command::Tell { project, body } => {
            let body = match body {
                Body::Given(body) => body,
                Body::Stdin => json_arg(&read_stdin()?)?,
            };
            let answer = client::tell(&socket_path(&project), &project, &body).await;
            print(
                answer.map_err(|e| e.to_string())?,
                &format!("told {project}"),
            )
        }
        Command::Reveal { moniker, state } => {
            let socket = socket_path(moniker.project());
            let answer = client::reveal(&socket, &moniker, state.as_ref()).await;
            print(
                answer.map_err(|e| e.to_string())?,
                &format!("revealed {moniker}"),
            )
        }
    }
}

async fn serve(project: String, build: PathBuf, policy: Policy) -> Result<(), String> {
    let socket = socket_path(&project);
    let host = Host::bind(Config {
        project,
        build: Build::Folder(build),
        policy,
        socket,
    })
    .await
    .map_err(|e| e.to_string())?;
    println!("open the page at {}", host.page_url());
    println!("stop it with Ctrl-C");
    host.serve()
        .await
        .map_err(|e| format!("the host stopped: {e}"))
}

fn read_stdin() -> Result<String, String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| format!("reading stdin: {e}"))?;
    Ok(text)
}

/// The page's answer, printed as `shala` prints one.
fn print(answer: Answer, done: &str) -> Result<(), String> {
    if !answer.ok {
        return Err(format!("the page could not do it: {}", answer.message));
    }
    println!("{done}");
    println!("message: {}", answer.message);
    if let Some(result) = answer.result {
        println!("result: {result}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(String::from).collect()
    }

    #[test]
    fn serve_takes_origins_and_wasm_in_any_order() {
        let command = parse(&args(
            "serve demo dist --wasm --connect http://localhost:3381",
        ));
        let Ok(Command::Serve {
            connect,
            wasm,
            remote_images,
            ..
        }) = command
        else {
            panic!("{command:?}");
        };
        assert!(wasm);
        assert!(!remote_images);
        assert_eq!(
            connect,
            vec!["http://localhost:3381".parse().expect("origin")]
        );
    }

    #[test]
    fn serve_takes_images() {
        let command = parse(&args("serve demo dist --images"));
        let Ok(Command::Serve { remote_images, .. }) = command else {
            panic!("{command:?}");
        };
        assert!(remote_images);
    }

    #[test]
    fn reveal_takes_state_before_or_after_the_moniker() {
        let after = parse(&args(r#"reveal demo://demo/x --state {"n":1}"#));
        let before = parse(&args(r#"reveal --state {"n":1} demo://demo/x"#));
        let expected = Command::Reveal {
            moniker: "demo://demo/x".parse().expect("moniker"),
            state: Some(json!({ "n": 1 })),
        };
        assert_eq!(after, Ok(expected));
        assert_eq!(
            before,
            parse(&args(r#"reveal demo://demo/x --state {"n":1}"#))
        );
    }

    #[test]
    fn bad_arguments_are_refused_not_guessed() {
        for line in [
            "",
            "show demo",
            "tell demo",
            "tell demo {not-json",
            "serve demo dist --connect",
            "serve demo dist --connect http://Example.com/path",
            "serve demo dist --verbose",
            "reveal",
            "reveal demo",
            "reveal demo demo://demo/x",
            "reveal demo://demo/x --state",
        ] {
            assert!(parse(&args(line)).is_err(), "{line:?}");
        }
    }
}
