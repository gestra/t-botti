/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use log::warn;
use std::sync::Arc;
use tokio::sync::mpsc;
use ts3_query::*;
use yaml_rust::yaml::Yaml;

use crate::botaction::{ActionType, BotAction};
use crate::IrcChannel;

fn get_clients(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<Vec<String>, Ts3Error> {
    let mut client = QueryClient::new(format!("{}:{}", host, port))?;
    client.login(username, password)?;
    client.select_server_by_id(1)?;

    let clients_full = client.online_clients()?;
    let real_clients: Vec<String> = clients_full
        .iter()
        .filter_map(|c| {
            if c.client_type == 0 {
                let chars = c.client_nickname.chars();
                let mut out = String::new();
                let max_len = 2;
                for c in chars {
                    out.push(c);
                    if out.len() >= max_len {
                        break;
                    }
                }
                Some(out)
            } else {
                None
            }
        })
        .collect();

    client.logout()?;

    Ok(real_clients)
}

fn generate_msg(nicks: Vec<String>) -> String {
    match nicks.len() {
        0 => "TS:ssä ei ole ketään".to_owned(),
        1 => format!("TS:ssä on 1 käyttäjä: {}", nicks[0]),
        _ => format!("TS:ssä on {} käyttäjää: {}", nicks.len(), nicks.join(", ")),
    }
}

pub async fn command_ts(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    config: Arc<Yaml>,
) {
    let get_conf = || -> Option<(String, u16, String, String)> {
        let host = config["teamspeak3"]["host"].as_str()?.to_owned();
        let port = config["teamspeak3"]["port"].as_i64().unwrap_or(10011) as u16;
        let username = config["teamspeak3"]["serverquery_login"]
            .as_str()?
            .to_owned();
        let password = config["teamspeak3"]["serverquery_password"]
            .as_str()?
            .to_owned();

        Some((host, port, username, password))
    };

    let msg = if let Some((host, port, username, password)) = get_conf() {
        match get_clients(&host, port, &username, &password) {
            Ok(v) => generate_msg(v),
            Err(e) => {
                warn!("Error when fetching teamspeak clients: {:?}", e);
                "Error when fetching teamspeak clients".to_owned()
            }
        }
    } else {
        warn!("Unable to get teamspeak3 configuration from config file");
        "Teamspeak 3 not configured properly".to_owned()
    };

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}
