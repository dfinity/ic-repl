use crate::exp::{str_to_principal, Exp};
use crate::token::{Token, Tokenizer};
use ansi_term::Color;
use candid::{
    check_prog,
    parser::configs::Configs,
    parser::value::{IDLField, IDLValue, VariantValue},
    pretty_parse,
    types::{Function, Label, Type},
    Decode, Encode, IDLArgs, IDLProg, Principal, TypeEnv,
};
use ic_agent::{Agent, Identity};
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
use tokio::runtime::Runtime;

#[derive(Default, Clone)]
pub struct CanisterMap(pub BTreeMap<Principal, CanisterInfo>);
#[derive(Default, Clone)]
pub struct IdentityMap(pub BTreeMap<String, std::sync::Arc<dyn Identity>>);
#[derive(Default, Clone)]
pub struct Env(pub BTreeMap<String, IDLValue>);
#[derive(Default, Clone)]
pub struct FuncEnv(pub BTreeMap<String, (Vec<String>, Vec<crate::command::Command>)>);
#[derive(Clone)]
pub struct CanisterInfo {
    pub env: TypeEnv,
    pub methods: BTreeMap<String, Function>,
    pub init: Option<Vec<Type>>,
}
#[derive(Clone)]
pub enum OfflineOutput {
    Json,
    Ascii(String),
    Png(String),
    PngNoUrl,
    AsciiNoUrl,
}
impl CanisterMap {
    pub fn get(&mut self, agent: &Agent, id: &Principal) -> anyhow::Result<&CanisterInfo> {
        if !self.0.contains_key(id) {
            let info = fetch_actor(agent, id)?;
            self.0.insert(*id, info);
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
    pub offline: Option<OfflineOutput>,
    pub canister_map: RefCell<CanisterMap>,
    pub identity_map: IdentityMap,
    pub current_identity: String,
    pub agent_url: String,
    pub agent: Agent,
    pub config: Configs,
    pub env: Env,
    pub func_env: FuncEnv,
    pub base_path: std::path::PathBuf,
    pub history: Vec<String>,
}

impl MyHelper {
    pub fn spawn(&self) -> Self {
        MyHelper {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            hinter: HistoryHinter {},
            colored_prompt: "".to_owned(),
            validator: MatchingBracketValidator::new(),
            history: Vec::new(),
            config: Configs::from_dhall("{=}").unwrap(),
            canister_map: self.canister_map.clone(),
            identity_map: self.identity_map.clone(),
            current_identity: self.current_identity.clone(),
            env: self.env.clone(),
            func_env: self.func_env.clone(),
            base_path: self.base_path.clone(),
            agent: self.agent.clone(),
            agent_url: self.agent_url.clone(),
            offline: self.offline.clone(),
        }
    }
    pub fn new(agent: Agent, agent_url: String, offline: Option<OfflineOutput>) -> Self {
        let mut res = MyHelper {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            hinter: HistoryHinter {},
            colored_prompt: "".to_owned(),
            validator: MatchingBracketValidator::new(),
            canister_map: RefCell::new(CanisterMap::default()),
            identity_map: IdentityMap::default(),
            current_identity: "anon".to_owned(),
            config: Configs::from_dhall("{=}").unwrap(),
            env: Env::default(),
            func_env: FuncEnv::default(),
            base_path: std::env::current_dir().unwrap(),
            history: Vec::new(),
            agent,
            agent_url,
            offline,
        };
        res.fetch_root_key_if_needed().unwrap();
        res.load_prelude().unwrap();
        res
    }
    fn load_prelude(&mut self) -> anyhow::Result<()> {
        self.preload_canister(
            "ic".to_string(),
            Principal::from_text("aaaaa-aa")?,
            include_str!("ic.did"),
        )?;
        if self.agent_url == "https://ic0.app" {
            // TODO remove when nns supports the tmp_hack method
            self.preload_canister(
                "nns".to_string(),
                Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")?,
                include_str!("governance.did"),
            )?;
            self.preload_canister(
                "ledger".to_string(),
                Principal::from_text("ryjl3-tyaaa-aaaaa-aaaba-cai")?,
                include_str!("ledger.did"),
            )?;
        }
        Ok(())
    }
    fn preload_canister(
        &mut self,
        name: String,
        id: Principal,
        did_file: &str,
    ) -> anyhow::Result<()> {
        let mut canister_map = self.canister_map.borrow_mut();
        canister_map
            .0
            .insert(id, did_to_canister_info(&name, did_file)?);
        self.env.0.insert(name, IDLValue::Principal(id));
        Ok(())
    }
    pub fn fetch_root_key_if_needed(&mut self) -> anyhow::Result<()> {
        if self.offline.is_none() && self.agent_url != "https://ic0.app" {
            let runtime = Runtime::new().expect("Unable to create a runtime");
            runtime.block_on(self.agent.fetch_root_key())?;
        };
        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone)]
enum Partial {
    Call(Principal, String),
    Val(IDLValue, String),
}

fn partial_parse(line: &str, pos: usize, helper: &MyHelper) -> Option<(usize, Partial)> {
    let (start, _) = extract_word(line, pos, None, b" ");
    let iter = Tokenizer::new(&line[start..pos]);
    let mut tokens = Vec::new();
    let mut pos_start = 0;
    for v in iter {
        let v = v.ok()?;
        if pos_start == 0
            && matches!(
                v.1,
                Token::Equals | Token::TestEqual | Token::SubEqual | Token::NotEqual
            )
        {
            pos_start = v.2;
        }
        let tok = if let Token::Text(id) = v.1 {
            Token::Id(id)
        } else {
            v.1
        };
        tokens.push((v.0, tok));
    }
    match tokens.as_slice() {
        [(_, Token::Id(id))] => match str_to_principal(id, helper) {
            Ok(id) => Some((pos, Partial::Call(id, "".to_string()))),
            Err(_) => parse_value(&line[..pos], start, pos, helper),
        },
        [(_, Token::Id(id)), (pos_tail, Token::Dot)]
        | [(_, Token::Id(id)), (pos_tail, Token::Dot), (_, _)] => {
            match str_to_principal(id, helper) {
                Ok(id) => Some((
                    start + pos_tail,
                    Partial::Call(id, line[start + pos_tail + 1..pos].to_string()),
                )),
                Err(_) => parse_value(&line[..pos], start + pos_start, start + pos_tail, helper),
            }
        }
        [.., (_, Token::RSquare)] | [.., (_, Token::Question)] => {
            parse_value(&line[..pos], start + pos_start, pos, helper)
        }
        [.., (pos_tail, Token::Dot)]
        | [.., (pos_tail, Token::Dot), (_, _)]
        | [.., (pos_tail, Token::LSquare)]
        | [.., (pos_tail, Token::LSquare), (_, Token::Decimal(_))] => {
            parse_value(&line[..pos], start + pos_start, start + pos_tail, helper)
        }
        _ => None,
    }
}
fn parse_value(
    line: &str,
    start: usize,
    end: usize,
    helper: &MyHelper,
) -> Option<(usize, Partial)> {
    let v = line[start..end].parse::<Exp>().ok()?.eval(helper).ok()?;
    Some((end, Partial::Val(v, line[end..].to_string())))
}
fn match_selector(v: &IDLValue, prefix: &str) -> Vec<Pair> {
    match v {
        IDLValue::Opt(_) => vec![Pair {
            display: "?".to_string(),
            replacement: "?".to_string(),
        }],
        IDLValue::Vec(vs) => vec![
            Pair {
                display: "vec".to_string(),
                replacement: "".to_string(),
            },
            Pair {
                display: format!("index should be less than {}", vs.len()),
                replacement: "".to_string(),
            },
        ],
        IDLValue::Record(fs) => fs.iter().filter_map(|f| match_field(f, prefix)).collect(),
        IDLValue::Variant(VariantValue(f, _)) => {
            if let Some(pair) = match_field(f, prefix) {
                vec![pair]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}
fn match_field(f: &IDLField, prefix: &str) -> Option<Pair> {
    match &f.id {
        Label::Named(name)
            if prefix.is_empty() || prefix.starts_with('.') && name.starts_with(&prefix[1..]) =>
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
        match partial_parse(line, pos, self) {
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

fn hint_method(line: &str, pos: usize, helper: &MyHelper) -> Option<String> {
    let start = line.rfind("encode").or_else(|| line.rfind("call"))?;
    let arg_pos = line[start..].find('(').unwrap_or(pos);
    match partial_parse(line, arg_pos, helper) {
        Some((_, Partial::Call(canister_id, method))) => {
            let mut map = helper.canister_map.borrow_mut();
            let info = map.get(&helper.agent, &canister_id).ok()?;
            let func = info.methods.get(&method)?;
            let given_args = line[arg_pos..].matches(',').count();
            let value = random_value(&info.env, &func.args, given_args, &helper.config).ok()?;
            Some(value)
        }
        _ => None,
    }
}

impl Hinter for MyHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() {
            return None;
        }
        hint_method(line, pos, self).or_else(|| self.hinter.hint(line, pos, ctx))
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
    given_args: usize,
    config: &Configs,
) -> candid::Result<String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let seed: Vec<_> = (0..2048).map(|_| rng.gen::<u8>()).collect();
    let result = IDLArgs::any(&seed, config, env, tys)?;
    Ok(if given_args > 0 {
        if given_args <= tys.len() {
            let mut res = String::new();
            for v in result.args[given_args..].iter() {
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
#[tokio::main]
pub async fn fetch_metadata(
    agent: &Agent,
    canister_id: Principal,
    sub_paths: &str,
) -> anyhow::Result<String> {
    use ic_agent::{hash_tree::Label, lookup_value};
    let mut path: Vec<Label> = vec!["canister".into(), canister_id.into()];
    path.extend(sub_paths.split('/').map(|str| str.into()));
    let cert = agent
        .read_state_raw(vec![path.clone()], canister_id)
        .await?;
    let response = lookup_value(&cert, path).map(<[u8]>::to_vec)?;
    Ok(String::from_utf8_lossy(&response).to_string())
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
    let init = find_init_args(&env, &actor);
    Ok(CanisterInfo { env, methods, init })
}

fn find_init_args(env: &TypeEnv, actor: &Type) -> Option<Vec<Type>> {
    match actor {
        Type::Var(id) => find_init_args(env, env.find_type(id).ok()?),
        Type::Class(init, _) => Some(init.to_vec()),
        _ => None,
    }
}

#[test]
fn test_partial_parse() -> anyhow::Result<()> {
    let url = "https://ic0.app".to_string();
    let agent = Agent::builder()
        .with_transport(
            ic_agent::agent::http_transport::ReqwestHttpReplicaV2Transport::create(url.clone())?,
        )
        .build()?;
    let mut helper = MyHelper::new(agent, url, None);
    helper.env.0.insert(
        "a".to_string(),
        "opt record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}".parse::<IDLValue>()?,
    );
    let ic0 = Principal::from_text("aaaaa-aa")?;
    helper
        .env
        .0
        .insert("ic0".to_string(), IDLValue::Principal(ic0));
    assert_eq!(partial_parse("call x", 6, &helper), None);
    assert_eq!(
        partial_parse("let id = call \"aaaaa-aa\"", 24, &helper).unwrap(),
        (24, Partial::Call(ic0, "".to_string()))
    );
    assert_eq!(
        partial_parse("let id = call \"aaaaa-aa\".", 25, &helper).unwrap(),
        (24, Partial::Call(ic0, "".to_string()))
    );
    assert_eq!(
        partial_parse("let id = call \"aaaaa-aa\".t", 26, &helper).unwrap(),
        (24, Partial::Call(ic0, "t".to_string()))
    );
    assert_eq!(
        partial_parse("let id = encode ic0", 19, &helper).unwrap(),
        (19, Partial::Call(ic0, "".to_string()))
    );
    assert_eq!(
        partial_parse("let id = encode ic0.", 20, &helper).unwrap(),
        (19, Partial::Call(ic0, "".to_string()))
    );
    assert_eq!(
        partial_parse("let id = encode ic0.t", 21, &helper).unwrap(),
        (19, Partial::Call(ic0, "t".to_string()))
    );
    assert_eq!(
        partial_parse("let id = a", 10, &helper).unwrap(),
        (
            10,
            Partial::Val(
                "opt record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}"
                    .parse::<IDLValue>()?,
                "".to_string()
            )
        )
    );
    assert_eq!(partial_parse("let id = a.f1.", 14, &helper), None);
    assert_eq!(
        partial_parse("let id =a?", 10, &helper).unwrap(),
        (
            10,
            Partial::Val(
                "record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}".parse::<IDLValue>()?,
                "".to_string()
            )
        )
    );
    assert_eq!(
        partial_parse("let id=a?.", 10, &helper).unwrap(),
        (
            9,
            Partial::Val(
                "record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}".parse::<IDLValue>()?,
                ".".to_string()
            )
        )
    );
    assert_eq!(
        partial_parse("let id = a?.f1", 14, &helper).unwrap(),
        (
            11,
            Partial::Val(
                "record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}".parse::<IDLValue>()?,
                ".f1".to_string()
            )
        )
    );
    assert_eq!(
        partial_parse("let id = a?[0", 13, &helper).unwrap(),
        (
            11,
            Partial::Val(
                "record { variant {b=vec{1;2;3}}; 42; f1=42;42=35;a1=30}".parse::<IDLValue>()?,
                "[0".to_string()
            )
        )
    );
    assert_eq!(
        partial_parse("let id = a?[0]", 14, &helper).unwrap(),
        (
            14,
            Partial::Val(
                "variant {b=vec{1;2;3}}".parse::<IDLValue>()?,
                "".to_string()
            )
        )
    );
    assert_eq!(
        partial_parse("let id = a?[0].", 15, &helper).unwrap(),
        (
            14,
            Partial::Val(
                "variant {b=vec{1;2;3}}".parse::<IDLValue>()?,
                ".".to_string()
            )
        )
    );
    Ok(())
}
