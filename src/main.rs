use clap::Parser;
use ic_agent::agent::http_transport::ReqwestTransport;
use ic_agent::Agent;
use rustyline::error::ReadlineError;
use rustyline::CompletionType;

mod account_identifier;
mod command;
mod error;
mod exp;
mod grammar;
mod helper;
mod offline;
mod profiling;
mod selector;
mod token;
mod utils;
use crate::command::Command;
use crate::error::pretty_parse;
use crate::helper::{MyHelper, OfflineOutput};

fn unwrap<T, E, F>(v: Result<T, E>, f: F)
where
    E: std::fmt::Debug,
    F: FnOnce(T),
{
    match v {
        Ok(res) => f(res),
        Err(e) => eprintln!("Error: {e:?}"),
    }
}

fn repl(opts: Opts) -> anyhow::Result<()> {
    let mut replica = opts.replica.unwrap_or_else(|| "local".to_string());
    let offline = if opts.offline {
        replica = "ic".to_string();
        let send_url = opts
            .url
            .unwrap_or_else(|| "https://qhmh2-niaaa-aaaab-qadta-cai.raw.icp0.io/?msg=".to_string());
        Some(match opts.format.as_deref() {
            None | Some("json") => OfflineOutput::Json,
            Some("ascii") => OfflineOutput::Ascii(send_url),
            Some("png") => OfflineOutput::Png(send_url),
            Some("png_no_url") => OfflineOutput::PngNoUrl,
            Some("ascii_no_url") => OfflineOutput::AsciiNoUrl,
            _ => unreachable!(),
        })
    } else {
        None
    };
    let url = match replica.as_str() {
        "local" => "http://localhost:4943/",
        "ic" => "https://icp0.io",
        url => url,
    };
    println!("Ping {url}...");
    let agent = Agent::builder()
        .with_transport(ReqwestTransport::create(url)?)
        .build()?;

    println!("Canister REPL");
    let config = rustyline::Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .build();
    let h = MyHelper::new(agent, url.to_string(), offline);
    if let Some(file) = opts.send {
        use crate::offline::{send_messages, Messages};
        let json = std::fs::read_to_string(file)?;
        let msgs = serde_json::from_str::<Messages>(&json)?;
        send_messages(&h, &msgs)?;
        return Ok(());
    }
    let mut rl = rustyline::Editor::with_config(config)?;
    rl.set_helper(Some(h));
    if rl.load_history("./.history").is_err() {
        eprintln!("No history found");
    }
    if let Some(file) = opts.config {
        let config = std::fs::read_to_string(file)?;
        rl.helper_mut().unwrap().config = candid_parser::configs::Configs::from_dhall(&config)?;
    }

    let enter_repl = opts.script.is_none() || opts.interactive;
    if let Some(file) = opts.script {
        let cmd = Command::Load(file);
        let helper = rl.helper_mut().unwrap();
        cmd.run(helper)?;
    }
    if enter_repl {
        let mut count = 1;
        loop {
            let identity = &rl.helper().unwrap().current_identity;
            let p = format!("{identity}@{replica} {count}> ");
            rl.helper_mut().unwrap().colored_prompt =
                format!("{}", console::style(&p).green().bold());
            let input = rl.readline(&p);
            match input {
                Ok(line) => {
                    rl.add_history_entry(&line)?;
                    unwrap(pretty_parse::<Command>("stdin", &line), |cmd| {
                        let helper = rl.helper_mut().unwrap();
                        helper.history.push(line.clone());
                        unwrap(cmd.run(helper), |_| {});
                    });
                }
                Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
                Err(err) => {
                    eprintln!("Error: {err:?}");
                    break;
                }
            }
            count += 1;
        }
        rl.save_history("./.history")?;
    }
    if opts.offline {
        let helper = rl.helper().unwrap();
        if !helper.messages.borrow().is_empty() {
            helper.dump_ingress()?;
        }
    }
    Ok(())
}

#[derive(Parser)]
#[clap(version, author)]
struct Opts {
    #[clap(short, long)]
    /// Specifies replica URL, possible values: local, ic, URL
    replica: Option<String>,
    #[clap(short, long, conflicts_with("replica"))]
    /// Offline mode to be run in air-gap machines. All signed messages will be stored in messages.json
    offline: bool,
    #[clap(short, long, requires("offline"), value_parser = ["ascii", "json", "png", "ascii_no_url", "png_no_url"])]
    /// Offline output format
    format: Option<String>,
    #[clap(short, long, requires("offline"))]
    /// Offline URL embeded in the QR code, only used in ascii or png format. Default value: "https://qhmh2-niaaa-aaaab-qadta-cai.raw.ic0.app/?msg="
    url: Option<String>,
    #[clap(short, long)]
    /// Specifies config file for Candid random value generation
    config: Option<String>,
    /// ic-repl script file
    script: Option<String>,
    #[clap(short, long, requires("script"))]
    /// Enter repl once the script is finished
    interactive: bool,
    #[clap(short, long, conflicts_with("script"), conflicts_with("offline"))]
    /// Send signed messages
    send: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    repl(opts)
}
