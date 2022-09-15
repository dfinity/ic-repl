use super::error::pretty_parse;
use super::exp::{str_to_principal, Exp};
use super::helper::{did_to_canister_info, fetch_metadata, MyHelper};
use super::token::{ParserError, Tokenizer};
use anyhow::{anyhow, Context};
use candid::{parser::configs::Configs, parser::value::IDLValue, Principal, TypeEnv};
use ic_agent::Agent;
use pretty_assertions::{assert_eq, assert_ne};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use terminal_size::{terminal_size, Width};

#[derive(Debug, Clone)]
pub struct Commands(pub Vec<Command>);
#[derive(Debug, Clone)]
pub enum Command {
    Config(String),
    Show(Exp),
    Let(String, Exp),
    Assert(BinOp, Exp, Exp),
    Export(String),
    Import(String, Principal, Option<String>),
    Load(String),
    Identity(String, Option<String>),
    Fetch(String, String),
    Func {
        name: String,
        args: Vec<String>,
        body: Vec<Command>,
    },
}
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone)]
pub enum BinOp {
    Equal,
    SubEqual,
    NotEqual,
}

impl Command {
    pub fn run(self, helper: &mut MyHelper) -> anyhow::Result<()> {
        match self {
            Command::Import(id, canister_id, did) => {
                if let Some(did) = &did {
                    let path = resolve_path(&helper.base_path, did);
                    let src = std::fs::read_to_string(&path)
                        .with_context(|| format!("Cannot read {:?}", path))?;
                    let info = did_to_canister_info(did, &src)?;
                    helper.canister_map.borrow_mut().0.insert(canister_id, info);
                }
                // TODO decide if it's a Service instead
                helper.env.0.insert(id, IDLValue::Principal(canister_id));
            }
            Command::Let(id, val) => {
                let v = val.eval(helper)?;
                helper.env.0.insert(id, v);
            }
            Command::Func { name, args, body } => {
                helper.func_env.0.insert(name, (args, body));
            }
            Command::Assert(op, left, right) => {
                let left = left.eval(helper)?;
                let right = right.eval(helper)?;
                match op {
                    BinOp::Equal => assert_eq!(left, right),
                    BinOp::SubEqual => {
                        if let (IDLValue::Text(left), IDLValue::Text(right)) = (&left, &right) {
                            assert!(left.contains(right));
                        } else {
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
                    }
                    BinOp::NotEqual => assert_ne!(left, right),
                }
            }
            Command::Config(conf) => helper.config = Configs::from_dhall(&conf)?,
            Command::Show(val) => {
                let time = Instant::now();
                let v = val.eval(helper)?;
                let duration = time.elapsed();
                println!("{}", v);
                helper.env.0.insert("_".to_string(), v);
                let width = if let Some((Width(w), _)) = terminal_size() {
                    w as usize
                } else {
                    80
                };
                println!("{:>width$}", format!("({:.2?})", duration), width = width);
            }
            Command::Fetch(id, path) => {
                let id = str_to_principal(&id, helper)?;
                let res = fetch_metadata(&helper.agent, id, &path)?;
                println!("{}", pretty_hex::pretty_hex(&res));
            }
            Command::Identity(id, opt_pem) => {
                use ic_agent::identity::{BasicIdentity, Secp256k1Identity};
                use ring::signature::Ed25519KeyPair;
                if let Some(pem_path) = &opt_pem {
                    let pem_path = resolve_path(&helper.base_path, pem_path);
                    match Secp256k1Identity::from_pem_file(&pem_path) {
                        Ok(identity) => {
                            helper
                                .identity_map
                                .0
                                .insert(id.to_string(), Arc::from(identity));
                        }
                        Err(_) => {
                            let identity = BasicIdentity::from_pem_file(&pem_path)?;
                            helper
                                .identity_map
                                .0
                                .insert(id.to_string(), Arc::from(identity));
                        }
                    }
                } else if helper.identity_map.0.get(&id).is_none() {
                    let rng = ring::rand::SystemRandom::new();
                    let pkcs8_bytes = Ed25519KeyPair::generate_pkcs8(&rng)?.as_ref().to_vec();
                    let keypair = Ed25519KeyPair::from_pkcs8(&pkcs8_bytes)?;
                    let identity = BasicIdentity::from_key_pair(keypair);
                    helper
                        .identity_map
                        .0
                        .insert(id.to_string(), Arc::from(identity));
                };
                let identity = helper.identity_map.0.get(&id).unwrap();
                let sender = identity.sender().map_err(|e| anyhow!("{}", e))?;
                println!("Current identity {}", sender);

                let agent = Agent::builder()
                    .with_transport(
                        ic_agent::agent::http_transport::ReqwestHttpReplicaV2Transport::create(
                            &helper.agent_url,
                        )?,
                    )
                    .with_arc_identity(identity.clone())
                    .build()?;
                helper.agent = agent;
                helper.fetch_root_key_if_needed()?;
                helper.current_identity = id.to_string();
                helper.env.0.insert(id, IDLValue::Principal(sender));
            }
            Command::Export(file) => {
                use std::io::{BufWriter, Write};
                let file = std::fs::File::create(file)?;
                let mut writer = BufWriter::new(&file);
                //for item in helper.history.iter() {
                for (id, val) in helper.env.0.iter() {
                    //writeln!(&mut writer, "{};", item)?;
                    writeln!(&mut writer, "let {} = {};", id, val)?;
                }
            }
            Command::Load(file) => {
                // TODO check for infinite loop
                let old_base = helper.base_path.clone();
                let path = resolve_path(&old_base, &file);
                let mut script = std::fs::read_to_string(&path)
                    .with_context(|| format!("Cannot read {:?}", path))?;
                if script.starts_with("#!") {
                    let line_end = script.find('\n').unwrap_or(0);
                    script.drain(..line_end);
                }
                let cmds = pretty_parse::<Commands>(&file, &script)?;
                helper.base_path = path.parent().unwrap().to_path_buf();
                for cmd in cmds.0.into_iter() {
                    //println!("> {:?}", cmd);
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

pub fn resolve_path(base: &Path, file: &str) -> PathBuf {
    let file = PathBuf::from(shellexpand::tilde(file).into_owned());
    if file.is_absolute() {
        file
    } else {
        base.join(file)
    }
}
