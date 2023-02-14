use crate::exp::MethodInfo;
use crate::helper::MyHelper;
use anyhow::anyhow;
use candid::{
    types::value::{IDLField, IDLValue},
    types::Label,
    Principal,
};
use ic_agent::Agent;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn ok_to_profile<'a>(helper: &'a MyHelper, info: &'a MethodInfo) -> bool {
    helper.offline.is_none()
        && info.profiling.is_some()
        && info.signature.as_ref().map(|s| s.1.is_query()) != Some(true)
}

#[tokio::main]
pub async fn get_cycles(agent: &Agent, canister_id: &Principal) -> anyhow::Result<i64> {
    use candid::{Decode, Encode};
    let mut builder = agent.query(canister_id, "__get_cycles");
    let bytes = builder
        .with_arg(Encode!()?)
        .with_effective_canister_id(*canister_id)
        .call()
        .await?;
    Ok(Decode!(&bytes, i64)?)
}

#[tokio::main]
pub async fn get_profiling(
    agent: &Agent,
    canister_id: &Principal,
    names: &BTreeMap<u16, String>,
    title: &str,
    filename: PathBuf,
) -> anyhow::Result<u64> {
    use candid::{Decode, Encode};
    let mut builder = agent.query(canister_id, "__get_profiling");
    let bytes = builder
        .with_arg(Encode!()?)
        .with_effective_canister_id(*canister_id)
        .call()
        .await?;
    let pairs = Decode!(&bytes, Vec<(i32, i64)>)?;
    if !pairs.is_empty() {
        render_profiling(pairs, names, title, filename)
    } else {
        eprintln!("empty trace");
        Ok(0)
    }
}

fn render_profiling(
    input: Vec<(i32, i64)>,
    names: &BTreeMap<u16, String>,
    title: &str,
    filename: PathBuf,
) -> anyhow::Result<u64> {
    use inferno::flamegraph::{from_reader, Options};
    let mut stack = Vec::new();
    let mut prefix = Vec::new();
    let mut result = Vec::new();
    let mut total = 0;
    let mut prev = None;
    for (id, count) in input.into_iter() {
        if id >= 0 {
            stack.push((id, count, 0));
            let name = match names.get(&(id as u16)) {
                Some(name) => name.clone(),
                None => "func_".to_string() + &id.to_string(),
            };
            prefix.push(name);
        } else {
            match stack.pop() {
                None => return Err(anyhow!("pop empty stack")),
                Some((start_id, start, children)) => {
                    if start_id != -id {
                        return Err(anyhow!("func id mismatch"));
                    }
                    let cost = count - start;
                    let frame = prefix.join(";");
                    prefix.pop().unwrap();
                    if let Some((parent, parent_cost, children_cost)) = stack.pop() {
                        stack.push((parent, parent_cost, children_cost + cost));
                    } else {
                        total += cost as u64;
                    }
                    match prev {
                        Some(prev) if prev == frame => {
                            // Add an empty spacer to avoid collapsing adjacent same-named calls
                            // See https://github.com/jonhoo/inferno/issues/185#issuecomment-671393504
                            result.push(format!("{};spacer 0", prefix.join(";")));
                        }
                        _ => (),
                    }
                    result.push(format!("{} {}", frame, cost - children));
                    prev = Some(frame);
                }
            }
        }
    }
    if !stack.is_empty() {
        result.push("incomplete_trace 10000".to_string());
        eprintln!("A trap occured or trace is too large");
    }
    //println!("Cost: {} Wasm instructions", total);
    let mut opt = Options::default();
    opt.count_name = "instructions".to_string();
    opt.title = title.to_string();
    opt.image_width = Some(1024);
    opt.flame_chart = true;
    opt.no_sort = true;
    // Reserve result order to make flamegraph from left to right.
    // See https://github.com/jonhoo/inferno/issues/236
    result.reverse();
    let logs = result.join("\n");
    let reader = std::io::Cursor::new(logs);
    println!("Flamegraph written to {}", filename.display());
    let mut writer = std::fs::File::create(&filename)?;
    from_reader(&mut opt, reader, &mut writer)?;
    Ok(total)
}

pub fn may_extract_profiling(result: IDLValue) -> (IDLValue, Option<i64>) {
    match result {
        IDLValue::Record(ref fs) => match fs.as_slice() {
            [IDLField {
                id: Label::Id(0),
                val,
            }, IDLField {
                id: Label::Id(1),
                val: IDLValue::Record(fs),
            }] => match fs.as_slice() {
                [IDLField {
                    id: Label::Named(lab),
                    val: IDLValue::Int64(cost),
                }] if lab == "__cost" => (val.clone(), Some(*cost)),
                _ => (result, None),
            },
            _ => (result, None),
        },
        _ => (result, None),
    }
}
