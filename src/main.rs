/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use tokio::sync::{mpsc, oneshot};

use yaml_rust::yaml::{Yaml, YamlLoader};

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::sync::Arc;

use log::{error, info};

#[macro_use]
extern crate lazy_static;

mod botaction;

mod blitzortung;
mod epic;
mod fmi;
mod gdq;
mod h33h3;
mod openweathermap;
mod ts3;
mod weather_db;
mod wolfram_alpha;

mod http_client;

mod rss;
use rss::rss_manager;

mod ircloop;
use ircloop::irc_loop;

mod timer;
use timer::timer_manager;

mod message_handler;
use message_handler::message_handler;

mod urltitle;

mod roll;

mod sahko;

mod tvmaze;

mod wikipedia;

#[derive(Debug, PartialEq, Eq)]
pub struct IrcChannel {
    network: String,
    channel: String,
}

#[derive(Debug)]
pub enum ClientQuery {
    IsAdmin(oneshot::Sender<bool>, String, String), // (sender, network, mask)
}

fn read_config_file() -> Result<String, ()> {
    let path = Path::new("config.yml");
    let mut file = match File::open(path) {
        Err(_) => {
            error!("Error when opening config.yml");
            error!("Copy config.yml.example as config.yml in the same directory as the executable and edit it to your liking.");
            return Err(());
        }
        Ok(file) => file,
    };

    let mut s = String::new();
    match file.read_to_string(&mut s) {
        Ok(_) => Ok(s),
        Err(_) => {
            error!("Error when reading config.yml");
            Err(())
        }
    }
}

fn get_config() -> Result<Vec<Yaml>, ()> {
    let file = match read_config_file() {
        Ok(f) => f,
        Err(_) => {
            return Err(());
        }
    };

    let docs = match YamlLoader::load_from_str(&file) {
        Ok(d) => d,
        Err(_) => {
            return Err(());
        }
    };

    Ok(docs)
}

#[tokio::main]
async fn main() -> Result<(), irc::error::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = match get_config() {
        Ok(y) => Arc::new(y[0].clone()),
        Err(_) => {
            error!("Could not get configuration. Exiting.");
            return Ok(());
        }
    };

    info!("Successfully read config file");

    let (botaction_tx, botaction_rx) = mpsc::channel(10);
    let (ircdata_tx, ircdata_rx) = mpsc::channel(10);
    let (timer_tx, timer_rx) = mpsc::channel(10);
    let (clientquery_tx, clientquery_rx) = mpsc::channel(10);

    let mut tasks = vec![];

    let c1 = config.clone();
    tasks.push(tokio::spawn(async move {
        irc_loop(ircdata_tx, botaction_rx, clientquery_rx, c1).await
    }));
    info!("Started irc_loop");

    let rssbot_tx = botaction_tx.clone();
    tasks.push(tokio::spawn(async move { rss_manager(rssbot_tx).await }));
    info!("Started rss_manager");

    let t_tx = botaction_tx.clone();
    tasks.push(tokio::spawn(
        async move { timer_manager(timer_rx, t_tx).await },
    ));
    info!("Started timer_manager");

    let messagehandler_tx = botaction_tx.clone();
    let c2 = config.clone();
    tasks.push(tokio::spawn(async move {
        message_handler(ircdata_rx, messagehandler_tx, timer_tx, clientquery_tx, c2).await
    }));
    info!("Started message_handler");

    for task in tasks {
        let _ = tokio::join!(task);
    }

    info!("All tasks finished");

    Ok(())
}
