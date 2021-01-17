/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use futures::prelude::*;
use irc::client::prelude::*;
use log::{debug, error};
use yaml_rust::yaml;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::ClientQuery;

pub async fn irc_loop(
    input_channel: mpsc::Sender<(String, Message)>,
    mut output_channel: mpsc::Receiver<BotAction>,
    mut clientquery_receiver: mpsc::Receiver<ClientQuery>,
    config: Arc<yaml::Yaml>,
) {
    let (common_ircdata_tx, mut common_ircdata_rx) = mpsc::channel(100);

    let networks = match config["networks"].as_vec() {
        Some(n) => n,
        None => {
            error!("No networks found in configuration!");
            return;
        }
    };

    let mut admins: HashMap<String, Vec<String>> = HashMap::new();

    let mut configs: HashMap<String, Config> = HashMap::new();
    for network in networks {
        let mut config = Config {
            ..Config::default()
        };
        
        let network_name = match network["network"].as_str() {
            Some(name) => name.to_owned(),
            None => {
                error!("Network must be given a name!");
                return;
            }
        };

        admins.insert(network_name.to_owned(), Vec::new());

        if let Some(nick) = network["nick"].as_str() {
            config.nickname = Some(nick.to_owned());
        }

        match network["server"].as_str() {
            Some(n) => {
                config.server = Some(n.to_owned());
            }
            None => {
                error!("Network {} has no server defined", network_name);
                return;
            }
        }

        if let Some(port) = network["port"].as_i64() {
            config.port = Some(port as u16);
        }

        if let Some(tls) = network["tls"].as_bool() {
            config.use_tls = Some(tls);
        }  else {
            config.use_tls = Some(false);
        }

        if let Some(channels) = network["channels"].as_vec() {
            let mut chan_vec = Vec::new();
            for channel in channels {
                if let Some(c) = channel.as_str() {
                    chan_vec.push(c.to_owned());
                }
            }
            config.channels = chan_vec;
        }

        if let Some(network_admins) = network["admins"].as_vec() {
            for admin in network_admins {
                if let Some(a) = admin.as_str() {
                    let v = admins.get_mut(&network_name).unwrap();
                    v.push(a.to_owned());
                }
            }
        }

        configs.insert(network_name, config);
    }

    let mut network_mpsc_senders: HashMap<String, mpsc::Sender<BotAction>> = HashMap::new();

    for (network, conf) in configs {
        let network_sender = common_ircdata_tx.clone();
        let (network_input_tx, mut network_input_rx) = mpsc::channel(10);
        network_mpsc_senders.insert(network.to_owned(), network_input_tx);

        tokio::spawn(async move {
            let mut client = Client::from_config(conf).await.unwrap();
            client.identify().unwrap();
            let mut stream = client.stream().unwrap();

            loop {
                tokio::select! {
                    Some(message) = stream.next() => {
                        if let Ok(m) = message {
                            debug!("Received message: {}", m);
                            network_sender.send((network.to_owned(), m)).await.unwrap();
                        }
                    }
                    Some(action) = network_input_rx.recv() => {
                        match action.action_type {
                            ActionType::Message(msg) => {
                                debug!("sending PRIVMSG {}", msg);
                                client.send_privmsg(action.target.channel, msg).unwrap();
                            }
                            ActionType::Action(msg) => {
                                debug!("sending ACTION {}", msg);
                                client.send_action(action.target.channel, msg).unwrap();
                            }
                        }
                    }
                }
            }
        });
    }

    loop {
        tokio::select! {
            Some((network, message)) = common_ircdata_rx.recv() => {
                input_channel.send((network.to_owned(), message)).await.unwrap();
            }
            Some(action) = output_channel.recv() => {
                if let Some(sender) = network_mpsc_senders.get(&action.target.network.to_owned()) {
                    sender.send(action).await.unwrap();
                }
            }
            Some(query) = clientquery_receiver.recv() => {
                match query {
                    ClientQuery::IsAdmin(response_channel, network, mask) => {
                        debug!("Querying if {} is owner on {}", mask, network);
                        let mut is_owner = false;
                        if let Some(network_admins) = admins.get(&network) {
                            for a in network_admins {
                                if a == &mask {
                                    is_owner = true;
                                    break;
                                }
                            }
                        }
                        debug!("is owner? {}", is_owner);
                        response_channel.send(is_owner).unwrap();
                    }
                }
            }
        }
    }
}
