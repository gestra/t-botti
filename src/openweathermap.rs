/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use irc::client::prelude::Prefix;
use yaml_rust::yaml;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::weather_db::get_location;
use crate::IrcChannel;

#[derive(Debug)]
struct WeatherData {
    place: Option<String>,
    temperature: Option<String>,
    wind: Option<String>,
    feels_like: Option<String>,
    humidity: Option<String>,
    cloudiness: Option<String>,
    description: Option<String>,
}

async fn get_json(city: &str, apikey: &str) -> reqwest::Result<String> {
    let baseurl = "https://api.openweathermap.org/data/2.5/weather";

    let json = HTTP_CLIENT
        .get(baseurl)
        .query(&[("units", "metric"), ("q", city), ("appid", apikey)])
        .send()
        .await?
        .text()
        .await?;

    Ok(json)
}

fn parse_json(json_text: &str) -> Result<WeatherData, String> {
    let mut place = None;
    let mut temperature = None;
    let mut wind = None;
    let mut feels_like = None;
    let mut humidity = None;
    let mut cloudiness = None;
    let mut description = None;

    let json: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(j) => j,
        Err(_) => {
            return Err("Error parsing JSON".to_owned());
        }
    };

    let country = json["sys"]["country"].as_str();
    let city = json["name"].as_str();
    if let Some(co) = country {
        if let Some(ci) = city {
            place = Some(format!("{}, {}", ci, co));
        }
    }

    if let Some(t) = json["main"]["temp"].as_f64() {
        temperature = Some(format!("{:.1}", t));
    }

    if let Some(w) = json["wind"]["speed"].as_f64() {
        wind = Some(format!("{:.1}", w));
    }

    if let Some(f) = json["main"]["feels_like"].as_f64() {
        feels_like = Some(format!("{:.1}", f));
    }

    if let Some(h) = json["main"]["humidity"].as_i64() {
        humidity = Some(format!("{}", h));
    }

    if let Some(c) = json["clouds"]["all"].as_i64() {
        cloudiness = Some(format!("{}", c));
    }

    if let Some(d) = json["weather"][0]["description"].as_str() {
        description = Some(d.to_string());
    }

    if !(place.is_some()
        || temperature.is_some()
        || wind.is_some()
        || feels_like.is_some()
        || humidity.is_some()
        || cloudiness.is_some()
        || description.is_some())
    {
        return Err("No data found".to_owned());
    }

    Ok(WeatherData {
        place,
        temperature,
        wind,
        feels_like,
        humidity,
        cloudiness,
        description,
    })
}

fn generate_msg(data: WeatherData) -> String {
    let mut msg = String::new();

    if let Some(p) = data.place {
        msg.push_str(&format!("{}: ", p));
    }
    if let Some(t) = data.temperature {
        msg.push_str(&format!("temperature: {}°C, ", t));
    }
    if let Some(f) = data.feels_like {
        msg.push_str(&format!("feels like: {}°C, ", f));
    }
    if let Some(w) = data.wind {
        msg.push_str(&format!("wind speed: {}m/s, ", w));
    }
    if let Some(h) = data.humidity {
        msg.push_str(&format!("humidity: {}%, ", h));
    }
    if let Some(c) = data.cloudiness {
        msg.push_str(&format!("cloudiness: {}%, ", c));
    }
    if let Some(d) = data.description {
        msg.push_str(&d);
    }

    if let Some(s) = msg.strip_suffix(", ") {
        msg = s.to_owned();
    }

    msg
}

pub async fn command_openweathermap(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    prefix: Option<Prefix>,
    params: &str,
    config: Arc<yaml::Yaml>,
) {
    let location = match params {
        "" => get_location(&prefix, &source.network),
        _ => params.to_owned(),
    };
    let msg;
    let apikey = match config["openweathermap"]["apikey"].as_str() {
        Some(a) => a,
        _ => {
            return;
        }
    };
    if let Ok(json) = get_json(&location, apikey).await {
        msg = match parse_json(&json) {
            Ok(data) => generate_msg(data),
            Err(_) => "Unable to get weather data".to_owned(),
        };
    } else {
        msg = "Unable to get weather data".to_owned();
    }

    let action = BotAction {
        target: source,
        action_type: ActionType::Message(msg),
    };

    bot_sender.send(action).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTJSON: &str = r###"{"coord":{"lon":8.55,"lat":47.3667},"weather":[{"id":800,"main":"Clear","description":"clear sky","icon":"01d"}],"base":"stations","main":{"temp":10.76,"feels_like":7.57,"temp_min":9,"temp_max":12.78,"pressure":1029,"humidity":53},"visibility":10000,"wind":{"speed":2.06,"deg":350},"clouds":{"all":0},"dt":1614604333,"sys":{"type":1,"id":6932,"country":"CH","sunrise":1614578776,"sunset":1614618620},"timezone":3600,"id":2657896,"name":"Zurich","cod":200}"###;

    #[test]
    fn owm() {
        let data = parse_json(TESTJSON).unwrap();
        assert_eq!(data.place, Some("Zurich, CH".to_owned()));
        assert_eq!(data.temperature, Some("10.8".to_owned()));
        assert_eq!(data.wind, Some("2.1".to_owned()));
        assert_eq!(data.feels_like, Some("7.6".to_owned()));
        assert_eq!(data.humidity, Some("53".to_owned()));
        assert_eq!(data.cloudiness, Some("0".to_owned()));
        assert_eq!(data.description, Some("clear sky".to_owned()));

        let msg = generate_msg(data);
        assert_eq!(msg, "Zurich, CH: temperature: 10.8°C, feels like: 7.6°C, wind speed: 2.1m/s, humidity: 53%, cloudiness: 0%, clear sky".to_owned());
    }
}
