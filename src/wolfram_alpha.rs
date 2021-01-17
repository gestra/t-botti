use log::error;
use std::sync::Arc;
use tokio::sync::mpsc;
use yaml_rust::yaml;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::IrcChannel;

async fn get_xml(query: &str, appid: &str) -> reqwest::Result<String> {
    let apiurl = "http://api.wolframalpha.com/v2/query";

    let xml = HTTP_CLIENT
        .get(apiurl)
        .query(&[("appid", appid), ("input", query)])
        .send()
        .await?
        .text()
        .await?;

    Ok(xml)
}

fn response_from_xml(xml: &str) -> Result<String, String> {
    let root = match xmltree::Element::parse(xml.as_bytes()) {
        Ok(r) => r,
        Err(_) => {
            error!("wolfram_alpha: Error parsing xml");
            return Err("Error parsing xml".to_owned());
        }
    };

    let mut interpretation: Option<String> = None;
    let mut answer: Option<String> = None;
    let mut didyoumean: Option<String> = None;

    for c in root.children {
        if let xmltree::XMLNode::Element(e) = c {
            if e.name == "pod" {
                if let Some(title) = e.attributes.get("title") {
                    if let Some(subpod) = e.get_child("subpod") {
                        match title.as_str() {
                            "Input interpretation" => {
                                if let Some(i) = subpod.get_child("plaintext") {
                                    if let Some(text) = i.get_text() {
                                        interpretation = Some(text.to_string());
                                    }
                                }
                            }
                            "Result" => {
                                if let Some(i) = subpod.get_child("plaintext") {
                                    if let Some(text) = i.get_text() {
                                        answer = Some(text.to_string());
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            } else if e.name == "didyoumeans" {
                if let Some(dym) = e.get_child("didyoumean") {
                    if let Some(text) = dym.get_text() {
                        didyoumean = Some(text.to_string());
                        break;
                    }
                }
            }
        }
    }

    let msg = if interpretation.is_some() && answer.is_some() {
        format!("{} = {}", interpretation.unwrap(), answer.unwrap())
    } else if didyoumean.is_some() {
        format!("Did you mean: {}", didyoumean.unwrap())
    } else {
        "Sorry, couldn't understand the question".to_owned()
    };

    Ok(msg)
}

pub async fn command_wa(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel, params: &str, config: Arc<yaml::Yaml>) {
    if let Some(apikey) = config["wolfram_alpha"]["apikey"].as_str() {
        if let Ok(xml) = get_xml(params, apikey).await {
            if let Ok(response) = response_from_xml(&xml) {
                let action = BotAction {
                    target: source,
                    action_type: ActionType::Message(response),
                };
                bot_sender.send(action).await.unwrap();
            }
        }
    }
}
