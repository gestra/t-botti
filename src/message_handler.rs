/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use irc::client::prelude::*;

use log::info;

use regex::Regex;

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use yaml_rust::yaml::Yaml;

use crate::blitzortung::command_ukkostutka;
use crate::botaction::{ActionType, BotAction};
use crate::epic::command_epic;
use crate::fmi::command_fmi;
use crate::gdq::command_gdq;
use crate::h33h3::handle_h33h3;
use crate::openweathermap::command_openweathermap;
use crate::roll::command_roll;
use crate::rss::command_rss;
use crate::sahko::command_sahko;
use crate::timer::{command_bigone, command_pizza, command_timer, TimerEvent};
use crate::ts3::command_ts;
use crate::tvmaze::command_ep;
use crate::urltitle::handle_url_titles;
use crate::weather_db::command_weatherset;
use crate::wikipedia::{command_wikipedia, command_wikipediafi};
use crate::wolfram_alpha::command_wa;
use crate::{ClientQuery, IrcChannel};

const COMMAND_PREFIX: char = '.';

lazy_static! {
    static ref RE_URL: Regex = Regex::new(r"(https?://[^ ]+)").unwrap();
}

async fn command_echo(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    params: &str,
    prefix: Option<Prefix>,
) {
    let msg_to_send = if let Some(Prefix::Nickname(nick, user, host)) = prefix {
        format!("{}!{}@{}: {}", nick, user, host, params)
    } else {
        format!("Echo: {}", params)
    };

    bot_sender
        .send(BotAction {
            target: source,
            action_type: ActionType::Message(msg_to_send),
        })
        .await
        .unwrap();
}

async fn is_admin(
    clientquery_sender: mpsc::Sender<ClientQuery>,
    prefix: Option<Prefix>,
    network: &str,
) -> bool {
    let mask = match prefix {
        Some(Prefix::Nickname(nick, user, host)) => format!("{}!{}@{}", nick, user, host),
        _ => {
            return false;
        }
    };

    let (admin_tx, admin_rx) = oneshot::channel();
    clientquery_sender
        .send(ClientQuery::IsAdmin(
            admin_tx,
            network.to_owned(),
            mask.to_owned(),
        ))
        .await
        .unwrap();

    let ret = matches!(admin_rx.await, Ok(true));

    info!("Checking whether {} is admin on {}: {}", mask, network, ret);

    ret
}

async fn handle_command(
    bot_sender: mpsc::Sender<BotAction>,
    timer_sender: mpsc::Sender<TimerEvent>,
    clientquery_sender: mpsc::Sender<ClientQuery>,
    source: IrcChannel,
    message: &str,
    prefix: Option<Prefix>,
    config: Arc<Yaml>,
) {
    let (command, params) = match message[1..].find(char::is_whitespace) {
        Some(i) => {
            let (c, mut p) = message[1..].split_at(i);
            p = p.trim_matches(char::is_whitespace);
            (c, p)
        }
        None => (&message[1..], ""),
    };

    info!("Command {} called by {:?}", command, prefix);

    match command {
        "echo" => {
            command_echo(bot_sender, source, params, prefix).await;
        }
        "timer" => {
            command_timer(bot_sender, timer_sender, source, params, prefix).await;
        }
        "pizza" => {
            command_pizza(bot_sender, timer_sender, source, prefix).await;
        }
        "bigone" => {
            command_bigone(bot_sender, timer_sender, source, prefix).await;
        }
        "rss" => {
            if is_admin(clientquery_sender, prefix, &source.network).await {
                command_rss(bot_sender, source, params).await;
            }
        }
        "sää" | "saa" | "fmi" => {
            command_fmi(bot_sender, source, prefix, params).await;
        }
        "weather" | "owm" => {
            command_openweathermap(bot_sender, source, prefix, params, config).await;
        }
        "weatherset" => {
            command_weatherset(bot_sender, source, prefix, params).await;
        }
        "roll" => {
            command_roll(bot_sender, source, params).await;
        }
        "ep" => {
            command_ep(bot_sender, source, params).await;
        }
        "wa" => {
            command_wa(bot_sender, source, params, config).await;
        }
        "wikipedia" => {
            command_wikipedia(bot_sender, source, params).await;
        }
        "wikipediafi" => {
            command_wikipediafi(bot_sender, source, params).await;
        }
        "epic" => {
            command_epic(bot_sender, source).await;
        }
        "ts" => {
            command_ts(bot_sender, source, config).await;
        }
        "ukkostutka" | "blitzortung" => {
            command_ukkostutka(bot_sender, source, params).await;
        }
        "agdq" | "sgdq" | "gdq" => {
            command_gdq(bot_sender, source).await;
        }
        "sähkö" | "sahko" => {
            command_sahko(bot_sender, source, config).await;
        }
        _ => {}
    }
}

pub async fn message_handler(
    mut receiver: mpsc::Receiver<(String, Message)>,
    sender: mpsc::Sender<BotAction>,
    timer_sender: mpsc::Sender<TimerEvent>,
    clientquery_sender: mpsc::Sender<ClientQuery>,
    config: Arc<Yaml>,
) {
    while let Some((network, message)) = receiver.recv().await {
        if let Command::PRIVMSG(_, msg) = &message.command {
            let msg_lower = msg.to_lowercase();
            let channel = match message.response_target() {
                Some(c) => c,
                _ => {
                    continue;
                }
            };

            if RE_URL.is_match(msg) {
                let snd = sender.clone();
                let msg_copy = String::from(msg);
                let source = IrcChannel {
                    network: network.to_owned(),
                    channel: channel.to_owned(),
                };
                tokio::spawn(async move {
                    handle_url_titles(snd, source, &msg_copy).await;
                });
            }

            if msg_lower.starts_with(COMMAND_PREFIX) {
                let prefix = match &message.prefix {
                    Some(Prefix::Nickname(nick, user, host)) => Some(Prefix::Nickname(
                        nick.to_owned(),
                        user.to_owned(),
                        host.to_owned(),
                    )),
                    _ => None,
                };
                let new_sender = sender.clone();
                let new_timer_sender = timer_sender.clone();
                let new_cq_sender = clientquery_sender.clone();
                let msg_copy = String::from(msg);
                let source = IrcChannel {
                    network: network.to_owned(),
                    channel: channel.to_owned(),
                };
                let cfg = config.clone();
                tokio::spawn(async move {
                    handle_command(
                        new_sender,
                        new_timer_sender,
                        new_cq_sender,
                        source,
                        &msg_copy,
                        prefix,
                        cfg,
                    )
                    .await;
                });
            }

            if msg_lower == "h33h3" {
                if let Some(Prefix::Nickname(nick, _, _)) = &message.prefix {
                    let nick_copy = nick.to_owned();
                    let new_sender = sender.clone();
                    let source = IrcChannel {
                        network: network.to_owned(),
                        channel: channel.to_owned(),
                    };
                    tokio::spawn(async move {
                        handle_h33h3(new_sender, source, &nick_copy).await;
                    });
                }
            }

            if msg_lower.contains("matt damon") {
                let s = sender.clone();
                let source = IrcChannel {
                    network: network.to_owned(),
                    channel: channel.to_owned(),
                };
                let mattdamon = "MATT DAMON".to_owned();
                let action = BotAction {
                    action_type: ActionType::Message(mattdamon),
                    target: source,
                };
                let _ = s.send(action).await;
            }
        }
    }
}
