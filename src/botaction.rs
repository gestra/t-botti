/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::IrcChannel;

#[derive(Debug, PartialEq)]
pub enum ActionType {
    Message(String),
    Action(String),
}

#[derive(Debug, PartialEq)]
pub struct BotAction {
    pub target: IrcChannel,
    pub action_type: ActionType,
}
