use crate::helper::{MyHelper, OfflineOutput};
use crate::utils::args_to_value;
use anyhow::{anyhow, Context, Result};
use candid::Principal;
use candid::{types::Function, IDLArgs, TypeEnv};
use ic_agent::{agent::RequestStatusResponse, Agent};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Ingress {
    pub call_type: String,
    pub request_id: Option<String>,
    pub content: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct RequestStatus {
    pub canister_id: Principal,
    pub request_id: String,
    pub content: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct IngressWithStatus {
    pub ingress: Ingress,
    pub request_status: Option<RequestStatus>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Messages(Vec<IngressWithStatus>);

static mut PNG_COUNTER: u32 = 0;

impl Ingress {
    pub fn parse(&self) -> Result<(Principal, Principal, String, Vec<u8>)> {
        use serde_cbor::Value;
        let cbor: Value = serde_cbor::from_slice(&hex::decode(&self.content)?)
            .context("Invalid cbor data in the content of the message.")?;
        if let Value::Map(m) = cbor {
            let cbor_content = m
                .get(&Value::Text("content".to_string()))
                .ok_or_else(|| anyhow!("Invalid cbor content"))?;
            if let Value::Map(m) = cbor_content {
                if let (
                    Some(Value::Bytes(sender)),
                    Some(Value::Bytes(canister_id)),
                    Some(Value::Text(method_name)),
                    Some(Value::Bytes(arg)),
                ) = (
                    m.get(&Value::Text("sender".to_string())),
                    m.get(&Value::Text("canister_id".to_string())),
                    m.get(&Value::Text("method_name".to_string())),
                    m.get(&Value::Text("arg".to_string())),
                ) {
                    let sender = Principal::try_from(sender)?;
                    let canister_id = Principal::try_from(canister_id)?;
                    return Ok((sender, canister_id, method_name.to_string(), arg.to_vec()));
                }
            }
        }
        Err(anyhow!("Invalid cbor content"))
    }
}
pub fn output_message(json: String, format: &OfflineOutput) -> Result<()> {
    match format {
        OfflineOutput::Json => println!("{json}"),
        _ => {
            use base64::{
                engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
                Engine,
            };
            use libflate::gzip;
            use qrcode::{render::unicode, QrCode};
            use std::io::Write;
            eprintln!("json length: {}", json.len());
            let mut encoder = gzip::Encoder::new(Vec::new())?;
            encoder.write_all(json.as_bytes())?;
            let zipped = encoder.finish().into_result()?;
            let engine = if matches!(format, OfflineOutput::PngNoUrl | OfflineOutput::AsciiNoUrl) {
                STANDARD_NO_PAD
            } else {
                URL_SAFE_NO_PAD
            };
            let base64 = engine.encode(zipped);
            eprintln!("base64 length: {}", base64.len());
            let msg = match format {
                OfflineOutput::Ascii(url) | OfflineOutput::Png(url) => url.to_owned() + &base64,
                _ => base64,
            };
            let code = QrCode::new(msg)?;
            match format {
                OfflineOutput::Ascii(_) | OfflineOutput::AsciiNoUrl => {
                    let img = code.render::<unicode::Dense1x2>().build();
                    println!("{img}");
                }
                OfflineOutput::Png(_) | OfflineOutput::PngNoUrl => {
                    let img = code.render::<image::Luma<u8>>().build();
                    let filename = unsafe {
                        PNG_COUNTER += 1;
                        format!("msg{PNG_COUNTER}.png")
                    };
                    img.save(&filename)?;
                    println!("QR code saved to {filename}");
                }
                _ => unreachable!(),
            }
        }
    };
    Ok(())
}
pub fn dump_ingress(msgs: &[IngressWithStatus]) -> Result<()> {
    use std::fs::File;
    use std::io::Write;
    let msgs = Messages(msgs.to_vec());
    let json = serde_json::to_string(&msgs)?;
    let mut file = File::create("messages.json")?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn send_messages(helper: &MyHelper, msgs: &Messages) -> Result<IDLArgs> {
    let len = msgs.0.len();
    let mut res = Vec::with_capacity(len);
    println!("Sending {} messages to {}", len, helper.agent_url);
    for (i, msg) in msgs.0.iter().enumerate() {
        print!("[{}/{}] ", i + 1, len);
        let args = send(helper, msg)?;
        res.push(args_to_value(args))
    }
    Ok(IDLArgs::new(&res))
}
pub fn send(helper: &MyHelper, msg: &IngressWithStatus) -> Result<IDLArgs> {
    let message = &msg.ingress;
    let (sender, canister_id, method_name, bytes) = message.parse()?;
    let meth = crate::exp::Method {
        canister: canister_id.to_string(),
        method: method_name.clone(),
    };
    let opt_func = meth.get_info(helper)?.signature;
    let args = if let Some((env, func)) = &opt_func {
        IDLArgs::from_bytes_with_types(&bytes, env, &func.args)?
    } else {
        IDLArgs::from_bytes(&bytes)?
    };
    println!("Sending {} call as {}:", message.call_type, sender);
    println!("  call \"{}\".{}{};", canister_id, method_name, args);
    println!("Do you want to send this message? [y/N]");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !["y", "yes"].contains(&input.to_lowercase().trim()) {
        return Err(anyhow!("Send abort"));
    }
    send_internal(&helper.agent, canister_id, msg, &opt_func)
}
#[tokio::main]
async fn send_internal(
    agent: &Agent,
    canister_id: Principal,
    message: &IngressWithStatus,
    opt_func: &Option<(TypeEnv, Function)>,
) -> Result<IDLArgs> {
    let content = hex::decode(&message.ingress.content)?;
    let response = match message.ingress.call_type.as_str() {
        "query" => agent.query_signed(canister_id, content).await?,
        "update" => {
            let request_id = agent.update_signed(canister_id, content).await?;
            println!("Request ID: 0x{}", String::from(request_id));
            let status = message
                .request_status
                .as_ref()
                .ok_or_else(|| anyhow!("Cannot get request status for update call"))?;
            if !(status.canister_id == canister_id && status.request_id == String::from(request_id))
            {
                return Err(anyhow!("request_id does match, cannot request status"));
            }
            let status = hex::decode(&status.content)?;
            let ic_agent::agent::Replied::CallReplied(blob) = async {
                loop {
                    match agent
                        .request_status_signed(&request_id, canister_id, status.clone())
                        .await?
                    {
                        RequestStatusResponse::Replied { reply } => return Ok(reply),
                        RequestStatusResponse::Rejected(response) => return Err(anyhow!(response)),
                        RequestStatusResponse::Done => return Err(anyhow!("No response")),
                        _ => println!("The request is being processed..."),
                    };
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
            .await?;
            blob
        }
        _ => unreachable!(),
    };
    let res = if let Some((env, func)) = &opt_func {
        IDLArgs::from_bytes_with_types(&response, env, &func.rets)?
    } else {
        IDLArgs::from_bytes(&response)?
    };
    println!("{}", res);
    Ok(res)
}
