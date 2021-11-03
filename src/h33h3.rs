/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use rand::prelude::*;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::IrcChannel;

struct H33h3Result {
    main_action: ActionType,
    extra_action: Option<ActionType>,
}

pub async fn handle_h33h3(bot_sender: mpsc::Sender<BotAction>, source: IrcChannel, nick: &str) {
    let result = {
        // Having the rng live past bot_sender.send seems to be a problem
        let mut rng = thread_rng();
        nbotti_h33h3(&mut rng, nick)
    };

    if let Some(extra) = result.extra_action {
        let target = IrcChannel {
            network: source.network.to_owned(),
            channel: source.channel.to_owned(),
        };
        let action = BotAction {
            action_type: extra,
            target,
        };
        let _ = bot_sender.send(action).await;
    }

    let action = BotAction {
        action_type: result.main_action,
        target: source,
    };
    let _ = bot_sender.send(action).await;
}

fn nbotti_h33h3<R: Rng + ?Sized>(rng: &mut R, nick: &str) -> H33h3Result {
    match rng.gen_range(0..=100) {
        23 | 55 => H33h3Result {
            main_action: ActionType::Message(format!(
                "GOOD DAY {}, YOU LOSE AT THE INTTER NETS",
                nick
            )),
            extra_action: None,
        },
        28 => H33h3Result {
            main_action: ActionType::Message("hngggg".to_owned()),
            extra_action: None,
        },
        29 => H33h3Result {
            main_action: ActionType::Message("h33h3".to_owned()),
            extra_action: None,
        },
        30 => H33h3Result {
            main_action: nbotti_kasipallo(rng),
            extra_action: Some(ActionType::Message("<W> har har har".to_owned())),
        },
        31 => H33h3Result {
            main_action: nbotti_kasipallo(rng),
            extra_action: Some(ActionType::Message("<W> HAR VITUN HAR".to_owned())),
        },
        _ => H33h3Result {
            main_action: nbotti_kasipallo(rng),
            extra_action: None,
        },
    }
}

fn nbotti_kasipallo<R: Rng + ?Sized>(rng: &mut R) -> ActionType {
    match rng.gen_range(1..=20) {
        1 => ActionType::Message(format!("{}", rng.gen_range(0..=4))),
        2 => ActionType::Message(".____________.".to_owned()),
        3 => ActionType::Message(format!("{}", rng.gen_range(0..=5))),
        4 => ActionType::Message(format!("{}", rng.gen_range(0..=2))),
        5 => ActionType::Message(format!("{}", rng.gen_range(0..=1))),
        6 => ActionType::Message(format!("{}", rng.gen_range(0..=3))),
        7 => ActionType::Action("am cry".to_owned()),
        8 => ActionType::Message("fail".to_owned()),
        9 | 10 => ActionType::Message("0".to_owned()),
        11 => ActionType::Message(format!("{}", rng.gen_range(0..=1))),
        12 => ActionType::Message(format!("{}", rng.gen_range(0..=2))),
        13 => ActionType::Message(format!("{}", rng.gen_range(0..=3))),
        14 => ActionType::Message(format!("{}", rng.gen_range(0..=4))),
        15 => ActionType::Message(format!("{}", rng.gen_range(0..=1))),
        16 => ActionType::Message("0".to_owned()),
        17 => ActionType::Message("::|".to_owned()), // First ':' gets eaten by something
        18 => ActionType::Message("h3-- not.".to_owned()),
        19 => ActionType::Message("0".to_owned()),
        20 => ActionType::Message(format!("{}", rng.gen_range(0..=5))),
        _ => panic!("RNG gen_range returned something outside the given range"),
    }
}
