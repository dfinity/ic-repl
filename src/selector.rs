use super::exp::Exp;
use super::helper::MyHelper;
use super::utils::as_u32;
use anyhow::{anyhow, Result};
use candid::{
    types::value::{IDLField, IDLValue, VariantValue},
    types::Label,
    utils::check_unique,
};

#[derive(Debug, Clone)]
pub enum Selector {
    Index(Exp),
    Field(String),
    Option,
    Map(String),
    Filter(String),
    Fold(Exp, String),
    Size, // Size is not required, but it is faster than using fold
}
impl Selector {
    fn to_label(&self, helper: &MyHelper) -> Result<Label> {
        Ok(match self {
            Selector::Index(e) => {
                let idx = as_u32(&e.clone().eval(helper)?)?;
                Label::Id(idx)
            }
            Selector::Field(name) => Label::Named(name.to_string()),
            _ => unreachable!(),
        })
    }
}
pub fn project(helper: &MyHelper, value: IDLValue, path: Vec<Selector>) -> Result<IDLValue> {
    let mut result = value;
    for head in path.into_iter() {
        match (result, head) {
            (IDLValue::Opt(opt), Selector::Option) => result = *opt,
            (IDLValue::Blob(b), Selector::Index(e)) => {
                let idx = as_u32(&e.eval(helper)?)?;
                result = IDLValue::Nat8(
                    *b.get(idx as usize)
                        .ok_or_else(|| anyhow!("idx out of bound"))?,
                )
            }
            (IDLValue::Vec(mut vs), Selector::Index(e)) => {
                let idx = as_u32(&e.eval(helper)?)? as usize;
                if idx < vs.len() {
                    result = vs.swap_remove(idx);
                } else {
                    return Err(anyhow!("{} out of bound {}", idx, vs.len()));
                }
            }
            (IDLValue::Text(s), Selector::Index(e)) => {
                let idx = as_u32(&e.eval(helper)?)? as usize;
                if idx < s.len() {
                    result = IDLValue::Text(s.chars().nth(idx).unwrap().to_string());
                } else {
                    return Err(anyhow!("{} out of bound {}", idx, s.len()));
                }
            }
            (IDLValue::Blob(b), Selector::Map(func)) => {
                let vs = b.into_iter().map(IDLValue::Nat8).collect();
                result = IDLValue::Vec(map(helper, vs, &func)?);
            }
            (IDLValue::Vec(vs), Selector::Map(func)) => {
                result = IDLValue::Vec(map(helper, vs, &func)?);
            }
            (IDLValue::Blob(b), Selector::Filter(func)) => {
                let vs = b.into_iter().map(IDLValue::Nat8).collect();
                result = IDLValue::Vec(filter(helper, vs, &func)?);
            }
            (IDLValue::Vec(vs), Selector::Filter(func)) => {
                result = IDLValue::Vec(filter(helper, vs, &func)?);
            }
            (IDLValue::Blob(b), Selector::Fold(init, func)) => {
                let vs = b.into_iter().map(IDLValue::Nat8).collect();
                result = fold(helper, init, vs, &func)?;
            }
            (IDLValue::Vec(vs), Selector::Fold(init, func)) => {
                result = fold(helper, init, vs, &func)?;
            }
            (IDLValue::Blob(b), Selector::Size) => {
                result = IDLValue::Nat(b.len().into());
            }
            (IDLValue::Vec(vs), Selector::Size) => {
                result = IDLValue::Nat(vs.len().into());
            }
            (IDLValue::Record(fs), Selector::Map(func)) => {
                let vs = from_fields(fs);
                let res = map(helper, vs, &func)?;
                result = IDLValue::Record(to_field(res)?);
            }
            (IDLValue::Record(fs), Selector::Filter(func)) => {
                let vs = from_fields(fs);
                let res = filter(helper, vs, &func)?;
                result = IDLValue::Record(to_field(res)?);
            }
            (IDLValue::Record(fs), Selector::Fold(init, func)) => {
                let vs = from_fields(fs);
                result = fold(helper, init, vs, &func)?;
            }
            (IDLValue::Record(fs), Selector::Size) => {
                result = IDLValue::Nat(fs.len().into());
            }
            (IDLValue::Text(s), Selector::Map(func)) => {
                let vs = from_text(s);
                let res = map(helper, vs, &func)?;
                result = IDLValue::Text(to_text(res)?);
            }
            (IDLValue::Text(s), Selector::Filter(func)) => {
                let vs = from_text(s);
                let res = filter(helper, vs, &func)?;
                result = IDLValue::Text(to_text(res)?);
            }
            (IDLValue::Text(s), Selector::Fold(init, func)) => {
                let vs = from_text(s);
                result = fold(helper, init, vs, &func)?;
            }
            (IDLValue::Text(s), Selector::Size) => {
                result = IDLValue::Nat(s.len().into());
            }
            (IDLValue::Record(fs), field @ (Selector::Index(_) | Selector::Field(_))) => {
                let id = field.to_label(helper)?;
                if let Some(v) = fs.into_iter().find(|f| f.id == id) {
                    result = v.val;
                } else {
                    return Err(anyhow!("record field {:?} not found", field));
                }
            }
            (
                IDLValue::Variant(VariantValue(f, _)),
                field @ (Selector::Index(_) | Selector::Field(_)),
            ) => {
                if field.to_label(helper)? == f.id {
                    result = f.val;
                } else {
                    return Err(anyhow!("variant field {:?} not found", field));
                }
            }
            (value, head) => {
                return Err(anyhow!(
                    "selector {:?} cannot be applied to {}",
                    head,
                    value
                ))
            }
        }
    }
    Ok(result)
}

fn from_fields(fs: Vec<IDLField>) -> Vec<IDLValue> {
    fs.into_iter()
        .map(|f| {
            IDLValue::Record(vec![
                IDLField {
                    id: Label::Id(0),
                    val: IDLValue::Text(format!("{}", f.id)),
                },
                IDLField {
                    id: Label::Id(1),
                    val: f.val,
                },
            ])
        })
        .collect()
}
fn from_text(s: String) -> Vec<IDLValue> {
    s.chars().map(|c| IDLValue::Text(c.to_string())).collect()
}
fn to_text(from: Vec<IDLValue>) -> Result<String> {
    use std::fmt::Write;
    let mut res = String::with_capacity(from.len());
    for v in from.into_iter() {
        if let IDLValue::Text(s) = v {
            write!(&mut res, "{s}")?;
        } else {
            return Err(anyhow!("expect function to return text"));
        }
    }
    Ok(res)
}
fn to_field(from: Vec<IDLValue>) -> Result<Vec<IDLField>> {
    let mut fs = Vec::with_capacity(from.len());
    for v in from.into_iter() {
        match v {
            IDLValue::Record(f) => match &f[..] {
                [IDLField {
                    val: IDLValue::Text(key),
                    ..
                }, IDLField { val, .. }] => {
                    let id = match key.parse::<u32>() {
                        Ok(id) => Label::Id(id),
                        Err(_) => Label::Named(key.to_string()),
                    };
                    fs.push(IDLField {
                        id,
                        val: val.clone(),
                    })
                }
                _ => return Err(anyhow!("expect function to return record {{ key; value }}")),
            },
            _ => return Err(anyhow!("expect function to return record {{ key; value }}")),
        }
    }
    fs.sort_unstable_by_key(|IDLField { id, .. }| id.get_id());
    check_unique(fs.iter().map(|f| &f.id))?;
    Ok(fs)
}

fn map(helper: &MyHelper, vs: Vec<IDLValue>, func: &str) -> Result<Vec<IDLValue>> {
    let mut new_helper = helper.spawn();
    let mut res = Vec::with_capacity(vs.len());
    for v in vs.into_iter() {
        new_helper.env.0.insert(String::new(), v);
        let arg = Exp::Path(String::new(), Vec::new());
        let exp = Exp::Apply(func.to_string(), vec![arg]);
        res.push(exp.eval(&new_helper)?);
    }
    Ok(res)
}

fn filter(helper: &MyHelper, vs: Vec<IDLValue>, func: &str) -> Result<Vec<IDLValue>> {
    let mut new_helper = helper.spawn();
    let mut res = Vec::with_capacity(vs.len());
    for v in vs.into_iter() {
        new_helper.env.0.insert(String::new(), v.clone());
        let arg = Exp::Path(String::new(), Vec::new());
        let exp = Exp::Apply(func.to_string(), vec![arg]);
        match exp.eval(&new_helper)? {
            IDLValue::Bool(false) => (),
            IDLValue::Bool(true) => res.push(v),
            _ => return Err(anyhow!("filter function needs to return bool")),
        }
    }
    Ok(res)
}

fn fold(helper: &MyHelper, init: Exp, vs: Vec<IDLValue>, func: &str) -> Result<IDLValue> {
    let init = init.eval(helper)?;
    let mut new_helper = helper.spawn();
    let mut acc = init;
    for v in vs.into_iter() {
        new_helper.env.0.insert(String::new(), v);
        let arg = Exp::Path(String::new(), Vec::new());
        new_helper.env.0.insert("_".to_string(), acc.clone());
        let accu = Exp::Path("_".to_string(), Vec::new());
        let exp = Exp::Apply(func.to_string(), vec![accu, arg]);
        acc = exp.eval(&new_helper)?;
    }
    Ok(acc)
}
