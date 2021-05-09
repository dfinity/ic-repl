use crate::command::extract_canister;
use crate::value::Value;
use ansi_term::Color;
use candid::{
    check_prog,
    parser::configs::Configs,
    parser::value::IDLValue,
    pretty_parse,
    types::{Function, Label, Type},
    Decode, Encode, IDLArgs, IDLProg, Principal, TypeEnv,
};
use ic_agent::Agent;
use rustyline::completion::{extract_word, Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{self, MatchingBracketValidator, Validator};
use rustyline::Context;
use rustyline_derive::Helper;
use std::borrow::Cow::{self, Borrowed, Owned};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct CanisterMap(pub BTreeMap<Principal, CanisterInfo>);
#[derive(Default)]
pub struct IdentityMap(pub BTreeMap<String, Vec<u8>>);
#[derive(Default)]
pub struct Env(pub BTreeMap<String, IDLValue>);
#[derive(Clone)]
pub struct CanisterInfo {
    pub env: TypeEnv,
    pub methods: BTreeMap<String, Function>,
}
impl CanisterMap {
    pub fn get(&mut self, agent: &Agent, id: &Principal) -> anyhow::Result<&CanisterInfo> {
        if !self.0.contains_key(id) {
            let info = fetch_actor(agent, id)?;
            self.0.insert(id.clone(), info);
        }
        Ok(self.0.get(id).unwrap())
    }
}
impl CanisterInfo {
    pub fn match_method(&self, meth: &str) -> Vec<Pair> {
        self.methods
            .iter()
            .filter(|(name, _)| name.starts_with(meth))
            .map(|(meth, func)| Pair {
                display: format!("{} : {}", meth, func),
                replacement: format!(".{}", meth),
            })
            .collect()
    }
}

#[derive(Helper)]
pub struct MyHelper {
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    validator: MatchingBracketValidator,
    hinter: HistoryHinter,
    pub colored_prompt: String,
    pub canister_map: RefCell<CanisterMap>,
    pub identity_map: IdentityMap,
    pub current_identity: String,
    pub agent_url: String,
    pub agent: Agent,
    pub config: Configs,
    pub env: Env,
    pub base_path: std::path::PathBuf,
    pub history: Vec<String>,
}

impl MyHelper {
    pub fn new(agent: Agent, agent_url: String) -> Self {
        let ic_did = include_str!("ic.did");
        let info = did_to_canister_info("ic.did", ic_did).unwrap();
        let mut canister_map = CanisterMap::default();
        canister_map
            .0
            .insert(Principal::from_text("aaaaa-aa").unwrap(), info);
        MyHelper {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            hinter: HistoryHinter {},
            colored_prompt: "".to_owned(),
            validator: MatchingBracketValidator::new(),
            canister_map: RefCell::new(canister_map),
            identity_map: IdentityMap::default(),
            current_identity: "anon".to_owned(),
            config: Configs::from_dhall("{=}").unwrap(),
            env: Env::default(),
            base_path: std::env::current_dir().unwrap(),
            history: Vec::new(),
            agent,
            agent_url,
        }
    }
}

#[derive(Debug)]
enum Partial {
    Call(Principal, String),
    Val(IDLValue, String),
}

fn extract_words(line: &str, pos: usize, helper: &MyHelper) -> Option<(usize, Partial)> {
    let (start, _) = extract_word(line, pos, None, b" ");
    let prev = &line[..start].trim_end();
    let (_, prev) = extract_word(prev, prev.len(), None, b" ");
    let is_call = matches!(prev, "call" | "encode");
    if is_call {
        let pos_tail = line[..pos].rfind('.').unwrap_or(pos);
        let tail = if pos_tail < pos {
            line[pos_tail + 1..pos].to_string()
        } else {
            String::new()
        };
        let id = &line[start..pos_tail];
        if id.starts_with('"') {
            let id = Principal::from_text(&id[1..id.len() - 1]).ok()?;
            Some((pos_tail, Partial::Call(id, tail)))
        } else {
            match helper.env.0.get(id)? {
                IDLValue::Principal(id) => Some((pos_tail, Partial::Call(id.clone(), tail))),
                _ => None,
            }
        }
    } else {
        let pos_tail = line[..pos].rfind(|c| c == '.' || c == '[').unwrap_or(pos);
        let v = line[start..pos_tail].parse::<Value>().ok()?;
        let v = v.eval(helper).ok()?;
        let tail = if pos_tail < pos {
            line[pos_tail..pos].to_string()
        } else {
            String::new()
        };
        Some((pos_tail, Partial::Val(v, tail)))
    }
}
fn match_selector(v: &IDLValue, prefix: &str) -> Vec<Pair> {
    match v {
        IDLValue::Opt(_) => vec![Pair {
            display: "?".to_string(),
            replacement: "?".to_string(),
        }],
        IDLValue::Record(fs) => fs
            .iter()
            .filter_map(|f| match &f.id {
                Label::Named(name)
                    if prefix.is_empty()
                        || prefix.starts_with('.') && name.starts_with(&prefix[1..]) =>
                {
                    Some(Pair {
                        display: format!(".{} = {}", name, f.val),
                        replacement: format!(".{}", name),
                    })
                }
                Label::Id(id) | Label::Unnamed(id)
                    if prefix.is_empty()
                        || prefix.starts_with('[') && id.to_string().starts_with(&prefix[1..]) =>
                {
                    Some(Pair {
                        display: format!("[{}] = {}", id, f.val),
                        replacement: format!("[{}]", id),
                    })
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Completer for MyHelper {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        match extract_words(line, pos, &self) {
            Some((pos, Partial::Call(canister_id, meth))) => {
                let mut map = self.canister_map.borrow_mut();
                Ok(match map.get(&self.agent, &canister_id) {
                    Ok(info) => (pos, info.match_method(&meth)),
                    Err(_) => (pos, Vec::new()),
                })
            }
            Some((pos, Partial::Val(v, rest))) => Ok((pos, match_selector(&v, &rest))),
            _ => self.completer.complete(line, pos, ctx),
        }
    }
}

impl Hinter for MyHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() {
            return None;
        }
        match extract_canister(line, pos, &self.env) {
            Some((_, canister_id, method, args)) => {
                let mut map = self.canister_map.borrow_mut();
                match map.get(&self.agent, &canister_id) {
                    Ok(info) => {
                        let func = info.methods.get(&method)?;
                        let value =
                            random_value(&info.env, &func.args, &args, &self.config).ok()?;
                        Some(value)
                    }
                    Err(_) => None,
                }
            }
            None => self.hinter.hint(line, pos, ctx),
        }
    }
}

impl Highlighter for MyHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(&self.colored_prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        let s = format!("{}", Color::White.dimmed().paint(hint));
        Owned(s)
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        self.highlighter.highlight_char(line, pos)
    }
}

impl Validator for MyHelper {
    fn validate(
        &self,
        ctx: &mut validate::ValidationContext,
    ) -> rustyline::Result<validate::ValidationResult> {
        self.validator.validate(ctx)
    }

    fn validate_while_typing(&self) -> bool {
        self.validator.validate_while_typing()
    }
}

fn random_value(
    env: &TypeEnv,
    tys: &[Type],
    given_args: &[Value],
    config: &Configs,
) -> candid::Result<String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let seed: Vec<_> = (0..2048).map(|_| rng.gen::<u8>()).collect();
    let result = IDLArgs::any(&seed, &config, env, &tys)?;
    Ok(if !given_args.is_empty() {
        if given_args.len() <= tys.len() {
            let mut res = String::new();
            for v in result.args[given_args.len()..].iter() {
                res.push_str(&format!(", {}", v));
            }
            res.push(')');
            res
        } else {
            "".to_owned()
        }
    } else {
        format!("{}", result)
    })
}

#[tokio::main]
async fn fetch_actor(agent: &Agent, canister_id: &Principal) -> anyhow::Result<CanisterInfo> {
    let response = agent
        .query(canister_id, "__get_candid_interface_tmp_hack")
        .with_arg(&Encode!()?)
        .call()
        .await?;
    let response = Decode!(&response, String)?;
    did_to_canister_info(&format!("did file for {}", canister_id), &response)
}

pub fn did_to_canister_info(name: &str, did: &str) -> anyhow::Result<CanisterInfo> {
    let ast = pretty_parse::<IDLProg>(name, did)?;
    let mut env = TypeEnv::new();
    let actor = check_prog(&mut env, &ast)?.unwrap();
    let methods = env
        .as_service(&actor)?
        .iter()
        .map(|(meth, ty)| {
            let func = env.as_func(ty).unwrap();
            (meth.to_owned(), func.clone())
        })
        .collect();
    Ok(CanisterInfo { env, methods })
}
