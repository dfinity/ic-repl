use super::error::pretty_parse;
use super::helper::{did_to_canister_info, MyHelper, NameEnv};
use super::token::{ParserError, Spanned, Tokenizer};
use super::value::Value;
use anyhow::{anyhow, Context};
use candid::{
    parser::configs::Configs, parser::value::IDLValue, types::Function, IDLArgs, Principal, TypeEnv,
};
use ic_agent::Agent;
use pretty_assertions::{assert_eq, assert_ne};
use std::path::{Path, PathBuf};
use std::time::Instant;
use terminal_size::{terminal_size, Width};

#[derive(Debug, Clone)]
pub struct Commands(pub Vec<Command>);
#[derive(Debug, Clone)]
pub enum Command {
    Call {
        canister: Spanned<String>,
        method: String,
        args: Vec<Value>,
        encode_only: bool,
    },
    Config(String),
    Show(Value),
    Let(String, Value),
    Assert(BinOp, Value, Value),
    Export(String),
    Import(String, Principal, Option<String>),
    Load(String),
    Identity(String),
}
#[derive(Debug, Clone)]
pub enum BinOp {
    Equal,
    SubEqual,
    NotEqual,
}

impl Command {
    pub fn run(self, helper: &mut MyHelper) -> anyhow::Result<()> {
        match self {
            Command::Call {
                canister,
                method,
                args,
                encode_only,
            } => {
                let try_id = Principal::from_text(&canister.value);
                let canister_id = match try_id {
                    Ok(ref id) => id,
                    Err(_) => helper
                        .canister_env
                        .0
                        .get(&canister.value)
                        .ok_or_else(|| anyhow!("Unknown canister {}", canister.value))?,
                };
                let agent = &helper.agent;
                let mut map = helper.canister_map.borrow_mut();
                let info = map.get(&agent, canister_id)?;
                let func = info
                    .methods
                    .get(&method)
                    .ok_or_else(|| anyhow!("no method {}", method))?;
                let mut values = Vec::new();
                for arg in args.into_iter() {
                    values.push(arg.eval(&helper)?);
                }
                let args = IDLArgs { args: values };
                if encode_only {
                    let bytes = args.to_bytes_with_types(&info.env, &func.args)?;
                    let res = IDLValue::Vec(bytes.into_iter().map(IDLValue::Nat8).collect());
                    println!("{}", res);
                    helper.env.0.insert("_".to_string(), res);
                    return Ok(());
                }
                let time = Instant::now();
                let res = call(&agent, canister_id, &method, &args, &info.env, &func)?;
                let duration = time.elapsed();
                println!("{}", res);
                let width = if let Some((Width(w), _)) = terminal_size() {
                    w as usize
                } else {
                    80
                };
                println!("{:>width$}", format!("({:.2?})", duration), width = width);
                // TODO multiple values
                for arg in res.args.into_iter() {
                    helper.env.0.insert("_".to_string(), arg);
                }
            }
            Command::Import(id, canister_id, did) => {
                if let Some(did) = &did {
                    let path = resolve_path(&helper.base_path, PathBuf::from(did));
                    let src = std::fs::read_to_string(&path)
                        .with_context(|| format!("Cannot read {:?}", path))?;
                    let info = did_to_canister_info(&did, &src)?;
                    helper
                        .canister_map
                        .borrow_mut()
                        .0
                        .insert(canister_id.clone(), info);
                }
                helper.canister_env.0.insert(id, canister_id);
            }
            Command::Let(id, val) => {
                let v = val.eval(&helper)?;
                helper.env.0.insert(id, v);
            }
            Command::Assert(op, left, right) => {
                let left = left.eval(&helper)?;
                let right = right.eval(&helper)?;
                match op {
                    BinOp::Equal => assert_eq!(left, right),
                    BinOp::SubEqual => {
                        let l_ty = left.value_ty();
                        let r_ty = right.value_ty();
                        let env = TypeEnv::new();
                        if let Ok(left) = left.annotate_type(false, &env, &r_ty) {
                            assert_eq!(left, right);
                        } else if let Ok(right) = right.annotate_type(false, &env, &l_ty) {
                            assert_eq!(left, right);
                        } else {
                            assert_eq!(left, right);
                        }
                    }
                    BinOp::NotEqual => assert_ne!(left, right),
                }
            }
            Command::Config(conf) => helper.config = Configs::from_dhall(&conf)?,
            Command::Show(val) => {
                let v = val.eval(&helper)?;
                println!("{}", v);
            }
            Command::Identity(id) => {
                use ic_agent::Identity;
                let keypair = if let Some(keypair) = helper.identity_map.0.get(&id) {
                    keypair.to_vec()
                } else {
                    let rng = ring::rand::SystemRandom::new();
                    let keypair = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)?
                        .as_ref()
                        .to_vec();
                    helper
                        .identity_map
                        .0
                        .insert(id.to_string(), keypair.clone());
                    keypair
                };
                let identity = ic_agent::identity::BasicIdentity::from_key_pair(
                    ring::signature::Ed25519KeyPair::from_pkcs8(&keypair)?,
                );
                let sender = identity.sender().map_err(|e| anyhow!("{}", e))?;
                println!("Current identity {}", sender);
                let agent = Agent::builder()
                    .with_transport(
                        ic_agent::agent::http_transport::ReqwestHttpReplicaV2Transport::create(
                            &helper.agent_url,
                        )?,
                    )
                    .with_identity(identity)
                    .build()?;
                {
                    let runtime =
                        tokio::runtime::Runtime::new().expect("Unable to create a runtime");
                    runtime.block_on(agent.fetch_root_key())?;
                }
                helper.agent = agent;
                helper.current_identity = id.to_string();
                helper.env.0.insert(id, IDLValue::Principal(sender));
            }
            Command::Export(file) => {
                use std::io::{BufWriter, Write};
                let file = std::fs::File::create(file)?;
                let mut writer = BufWriter::new(&file);
                for item in helper.history.iter() {
                    writeln!(&mut writer, "{};", item)?;
                }
            }
            Command::Load(file) => {
                // TODO check for infinite loop
                let old_base = helper.base_path.clone();
                let path = resolve_path(&old_base, PathBuf::from(&file));
                let mut script = std::fs::read_to_string(&path)
                    .with_context(|| format!("Cannot read {:?}", path))?;
                if script.starts_with("#!") {
                    let line_end = script.find('\n').unwrap_or(0);
                    script.drain(..line_end);
                }
                let cmds = pretty_parse::<Commands>(&file, &script)?;
                helper.base_path = path.parent().unwrap().to_path_buf();
                for cmd in cmds.0.into_iter() {
                    println!("> {:?}", cmd);
                    cmd.run(helper)?;
                }
                helper.base_path = old_base;
            }
        }
        Ok(())
    }
}

impl std::str::FromStr for Command {
    type Err = ParserError;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let lexer = Tokenizer::new(str);
        super::grammar::CommandParser::new().parse(lexer)
    }
}
impl std::str::FromStr for Commands {
    type Err = ParserError;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let lexer = Tokenizer::new(str);
        super::grammar::CommandsParser::new().parse(lexer)
    }
}

#[tokio::main]
async fn call(
    agent: &Agent,
    canister_id: &Principal,
    method: &str,
    args: &IDLArgs,
    env: &TypeEnv,
    func: &Function,
) -> anyhow::Result<IDLArgs> {
    let args = args.to_bytes_with_types(env, &func.args)?;
    let bytes = if func.is_query() {
        agent
            .query(canister_id, method)
            .with_arg(args)
            .with_effective_canister_id(canister_id.clone())
            .call()
            .await?
    } else {
        let waiter = delay::Delay::builder()
            .exponential_backoff(std::time::Duration::from_secs(1), 1.1)
            .timeout(std::time::Duration::from_secs(60 * 5))
            .build();
        agent
            .update(canister_id, method)
            .with_arg(args)
            .with_effective_canister_id(canister_id.clone())
            .call_and_wait(waiter)
            .await?
    };
    Ok(IDLArgs::from_bytes_with_types(&bytes, env, &func.rets)?)
}

// Return position at the end of principal, principal, method, args
pub fn extract_canister(
    line: &str,
    pos: usize,
    env: &NameEnv,
) -> Option<(usize, Principal, String, Vec<Value>)> {
    let command = line[..pos].parse::<Command>().ok()?;
    match command {
        Command::Call {
            canister,
            method,
            args,
            ..
        } => {
            let try_id = Principal::from_text(&canister.value);
            let canister_id = match try_id {
                Ok(id) => id,
                Err(_) => env.0.get(&canister.value)?.clone(),
            };
            Some((canister.span.end, canister_id, method, args))
        }
        _ => None,
    }
}

pub fn resolve_path(base: &Path, file: PathBuf) -> PathBuf {
    if file.is_absolute() {
        file
    } else {
        base.join(file)
    }
}
