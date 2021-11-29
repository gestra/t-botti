/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use irc::client::prelude::Prefix;
use std::collections::HashMap;

use chrono::prelude::*;
use tokio::sync::mpsc;

use crate::botaction::{ActionType, BotAction};
use crate::http_client::HTTP_CLIENT;
use crate::weather_db::get_location;
use crate::IrcChannel;

lazy_static! {
    // https://www.ilmatieteenlaitos.fi/latauspalvelun-pikaohje
    static ref WAWA: HashMap<u32, &'static str> = {
        let mut m = HashMap::new();
        m.insert(4, "auerta, savua tai ilmassa leijuvaa pölyä");
        m.insert(5, "auerta, savua tai ilmassa leijuvaa pölyä");
        m.insert(20, "sumua");
        m.insert(21, "sadetta");
        m.insert(22, "tihkusadetta tai lumijyväsiä");
        m.insert(23, "vesisadetta");
        m.insert(24, "lumisadetta");
        m.insert(25, "jäätävää vesisadetta tai jäätävää tihkua");
        m.insert(30, "sumua");
        m.insert(31, "sumua");
        m.insert(32, "sumua");
        m.insert(33, "sumua");
        m.insert(34, "sumua");
        m.insert(40, "sadetta");
        m.insert(41, "heikkoa tai kohtalaista sadetta");
        m.insert(42, "kovaa sadetta");
        m.insert(50, "tihkusadetta");
        m.insert(51, "heikkoa tihkusadetta");
        m.insert(52, "kohtalaista tihkusadetta");
        m.insert(53, "kovaa tihkusadetta");
        m.insert(54, "jäätävää heikkoa tihkusadetta");
        m.insert(55, "jäätävää kohtalaista tihkusadetta");
        m.insert(56, "jäätävää kovaa tihkusadetta");
        m.insert(60, "vesisadetta");
        m.insert(61, "heikkoa vesisadetta");
        m.insert(62, "kohtalaista vesisadetta");
        m.insert(63, "kovaa vesisadetta");
        m.insert(64, "jäätävää heikkoa vesisadetta");
        m.insert(65, "jäätävää kohtalaista vesisadetta");
        m.insert(66, "jäätävää kovaa vesisadetta");
        m.insert(70, "lumisadetta");
        m.insert(71, "heikkoa lumisadetta");
        m.insert(72, "kohtalaista lumisadetta");
        m.insert(73, "tiheää lumisadetta");
        m.insert(74, "heikkoa jääjyväsadetta");
        m.insert(75, "kohtalaista jääjyväsadetta");
        m.insert(76, "kovaa jääjyväsadetta");
        m.insert(77, "lumijyväsiä");
        m.insert(78, "jääkiteitä");
        m.insert(80, "kuuroja tai ajoittaista sadetta");
        m.insert(81, "heikkoja vesikuuroja");
        m.insert(82, "kohtalaisia vesikuuroja");
        m.insert(83, "kovia vesikuuroja");
        m.insert(84, "ankaria vesikuuroja");
        m.insert(85, "heikkoja lumikuuroja");
        m.insert(86, "kohtalaisia lumikuuroja");
        m.insert(87, "kovia lumikuuroja");
        m.insert(89, "raekuuroja");
        m
    };
}

#[derive(Debug)]
struct WeatherData {
    place: Option<String>,
    temperature: Option<String>,
    wind: Option<String>,
    gust: Option<String>,
    feels_like: Option<String>,
    humidity: Option<String>,
    cloudiness: Option<String>,
    wawa: Option<String>,
}

async fn get_xml(place: &str) -> reqwest::Result<String> {
    let starttime = Utc::now() - chrono::Duration::minutes(15);
    let timestamp = starttime.to_rfc3339_opts(SecondsFormat::Secs, true);

    let baseurl = "https://opendata.fmi.fi/wfs";

    let xml = HTTP_CLIENT
        .get(baseurl)
        .query(&[
            ("service", "WFS"),
            ("version", "2.0.0"),
            ("request", "getFeature"),
            (
                "storedquery_id",
                "fmi::observations::weather::timevaluepair",
            ),
            ("maxlocations", "1"),
            ("place", place),
            ("starttime", &timestamp),
        ])
        .send()
        .await?
        .text()
        .await?;

    Ok(xml)
}

fn parse_xml(xml: &str) -> Result<WeatherData, String> {
    fn get_value(element: &xmltree::Element) -> Option<String> {
        let last_point = element.children.last()?;
        if let xmltree::XMLNode::Element(ce) = last_point {
            if let Some(mtvp) = ce.get_child("MeasurementTVP") {
                if let Some(value) = mtvp.get_child("value") {
                    return Some(value.get_text()?.to_string());
                }
            }
        }

        None
    }

    fn calc_feels_like(temperature: f64, wind: f64) -> f64 {
        // https://fi.wikipedia.org/wiki/Pakkasen_purevuus#Uusi_kaava
        13.12 + 0.6215 * temperature - 13.956 * wind.powf(0.16) + 0.4867 * temperature * wind.powf(0.16)
    }

    let root = match xmltree::Element::parse(xml.as_bytes()) {
        Ok(r) => r,
        Err(_) => {
            return Err("Error parsing xml".to_owned());
        }
    };

    let mut place = None;
    let mut temperature = None;
    let mut wind = None;
    let mut gust = None;
    let mut feels_like = None;
    let mut humidity = None;
    let mut cloudiness = None;
    let mut wawa = None;

    if let Some(p) = root
        .get_child("member")
        .and_then(|m| m.get_child("PointTimeSeriesObservation"))
        .and_then(|p| p.get_child("featureOfInterest"))
        .and_then(|f| f.get_child("SF_SpatialSamplingFeature"))
        .and_then(|s| s.get_child("shape"))
        .and_then(|s| s.get_child("Point"))
        .and_then(|p| p.get_child("name"))
        .and_then(|n| n.get_text())
    {
        place = Some(p.to_string());
    }

    for c in root.children {
        if let xmltree::XMLNode::Element(ce) = c {
            if let Some(mts) = ce
                .get_child("PointTimeSeriesObservation")
                .and_then(|ptso| ptso.get_child("result"))
                .and_then(|result| result.get_child("MeasurementTimeseries"))
            {
                if let Some(id) = mts.attributes.get("id") {
                    if let Some(value) = get_value(mts) {
                        match id as &str {
                            "obs-obs-1-1-t2m" => {
                                if value != "NaN" {
                                    temperature = Some(value);
                                }
                            }
                            "obs-obs-1-1-ws_10min" => {
                                if value != "NaN" {
                                    wind = Some(value);
                                }
                            }
                            "obs-obs-1-1-wg_10min" => {
                                if value != "NaN" {
                                    gust = Some(value);
                                }
                            }
                            "obs-obs-1-1-rh" => {
                                if value != "NaN" {
                                    if let Some(i) = value.strip_suffix(".0") {
                                        humidity = Some(i.to_owned());
                                    } else {
                                        humidity = Some(value);
                                    }
                                }
                            }
                            "obs-obs-1-1-wawa" => {
                                if let Some(v) = value.strip_suffix(".0") {
                                    if let Ok(i) = v.parse::<u32>() {
                                        if let Some(d) = WAWA.get(&i) {
                                            wawa = Some(d.to_string());
                                        }
                                    }
                                }
                            }
                            "obs-obs-1-1-n_man" => {
                                if value != "NaN" {
                                    if let Some(i) = value.strip_suffix(".0") {
                                        cloudiness = Some(i.to_owned());
                                    } else {
                                        cloudiness = Some(value);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    if let Some(ref t) = temperature {
        if let Some(ref w) = wind {
            if let Ok(t_f) = t.parse::<f64>() {
                if t_f <= 10.0 {
                    if let Ok(w_f) = w.parse::<f64>() {
                        let f = calc_feels_like(t_f, w_f);
                        feels_like = Some(format!("{:.1}", f));
                    }
                }
            }
        }
    }

    if !(place.is_some()
        || temperature.is_some()
        || wind.is_some()
        || gust.is_some()
        || feels_like.is_some()
        || humidity.is_some()
        || cloudiness.is_some()
        || wawa.is_some())
    {
        return Err("Tietoja ei löytynyt".to_owned());
    }

    Ok(WeatherData {
        place,
        temperature,
        wind,
        gust,
        feels_like,
        humidity,
        cloudiness,
        wawa,
    })
}

fn generate_msg(data: WeatherData) -> String {
    let mut msg = String::new();

    if let Some(p) = data.place {
        msg.push_str(&format!("{}: ", p));
    }
    if let Some(t) = data.temperature {
        msg.push_str(&format!("lämpötila: {}°C, ", t));
    }
    if let Some(f) = data.feels_like {
        msg.push_str(&format!("tuntuu kuin: {}°C, ", f));
    }
    if let Some(w) = data.wind {
        msg.push_str(&format!("tuulen nopeus: {}m/s, ", w));
    }
    if let Some(g) = data.gust {
        msg.push_str(&format!("puuskat: {}m/s, ", g));
    }
    if let Some(h) = data.humidity {
        msg.push_str(&format!("ilman kosteus: {}%, ", h));
    }
    if let Some(c) = data.cloudiness {
        msg.push_str(&format!("pilvisyys: {}/8, ", c));
    }
    if let Some(w) = data.wawa {
        msg.push_str(&w);
    }

    if let Some(s) = msg.strip_suffix(", ") {
        msg = s.to_owned();
    }

    msg
}

pub async fn command_fmi(
    bot_sender: mpsc::Sender<BotAction>,
    source: IrcChannel,
    prefix: Option<Prefix>,
    params: &str,
) {
    let location = match params {
        "" => get_location(&prefix, &source.network),
        _ => params.to_owned(),
    };
    let msg;
    if let Ok(xml) = get_xml(&location).await {
        msg = match parse_xml(&xml) {
            Ok(data) => generate_msg(data),
            Err(e) => e,
        };
    } else {
        msg = "Tietojen haku ei onnistunut".to_owned();
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

    const FMI_XML: &str = r###"<?xml version="1.0" encoding="UTF-8"?>
<wfs:FeatureCollection timeStamp="2021-02-21T14:40:51Z" numberMatched="13" numberReturned="13" xmlns:wfs="http://www.opengis.net/wfs/2.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:om="http://www.opengis.net/om/2.0" xmlns:ompr="http://inspire.ec.europa.eu/schemas/ompr/3.0" xmlns:omso="http://inspire.ec.europa.eu/schemas/omso/3.0" xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:gmd="http://www.isotc211.org/2005/gmd" xmlns:gco="http://www.isotc211.org/2005/gco" xmlns:swe="http://www.opengis.net/swe/2.0" xmlns:gmlcov="http://www.opengis.net/gmlcov/1.0" xmlns:sam="http://www.opengis.net/sampling/2.0" xmlns:sams="http://www.opengis.net/samplingSpatial/2.0" xmlns:wml2="http://www.opengis.net/waterml/2.0" xmlns:target="http://xml.fmi.fi/namespace/om/atmosphericfeatures/1.0" xsi:schemaLocation="http://www.opengis.net/wfs/2.0 http://schemas.opengis.net/wfs/2.0/wfs.xsd         http://www.opengis.net/gmlcov/1.0 http://schemas.opengis.net/gmlcov/1.0/gmlcovAll.xsd         http://www.opengis.net/sampling/2.0 http://schemas.opengis.net/sampling/2.0/samplingFeature.xsd         http://www.opengis.net/samplingSpatial/2.0 http://schemas.opengis.net/samplingSpatial/2.0/spatialSamplingFeature.xsd         http://www.opengis.net/swe/2.0 http://schemas.opengis.net/sweCommon/2.0/swe.xsd         http://inspire.ec.europa.eu/schemas/ompr/3.0 http://inspire.ec.europa.eu/schemas/ompr/3.0/Processes.xsd         http://inspire.ec.europa.eu/schemas/omso/3.0 http://inspire.ec.europa.eu/schemas/omso/3.0/SpecialisedObservations.xsd         http://www.opengis.net/waterml/2.0 http://schemas.opengis.net/waterml/2.0/waterml2.xsd         http://xml.fmi.fi/namespace/om/atmosphericfeatures/1.0 http://xml.fmi.fi/schema/om/atmosphericfeatures/1.0/atmosphericfeatures.xsd">
   
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-7J086k2tiboEOaGvd5_rE0IS6auJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udY3Rlta23Tz56d2epl8dKxp2Gc2t3XbPzU.mHpp37uc4TW49cOzT08yd2bfE1ufTD00791Tzwy1ob.GXdkw9MLc59N_LLk49cvLzf05K0ws23S6db8XPLy7Yemnfu5unXfLh6aMvJ066aduXth2dcvDDp5NDpp25afTLwmaHTTty2t.7LWNVqQwA-">

		            <om:phenomenonTime>
        <gml:TimePeriod gml:id="time1-1-1">
          <gml:beginPosition>2021-02-21T14:30:00Z</gml:beginPosition>
          <gml:endPosition>2021-02-21T14:40:00Z</gml:endPosition>
        </gml:TimePeriod>
      </om:phenomenonTime>
      <om:resultTime>
        <gml:TimeInstant gml:id="time2-1-1">
          <gml:timePosition>2021-02-21T14:40:00Z</gml:timePosition>
        </gml:TimeInstant>
      </om:resultTime>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=t2m&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-t2m">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-t2m">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-t2m">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-t2m"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-t2m" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-t2m">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>-1.3</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-ztQhW6V1xZMwXHsjaR07ZCgI2sKJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udaHfnfYsNunc1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhg">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=ws_10min&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-ws_10min">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-ws_10min">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-ws_10min">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-ws_10min"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-ws_10min" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-ws_10min">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>6.5</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-p3eSeTKUySIRLzr01JyTMUxkvSGJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udaHfPfYsNunc1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhg">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=wg_10min&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-wg_10min">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-wg_10min">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-wg_10min">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-wg_10min"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-wg_10min" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-wg_10min">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>9.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-Z2HMeOWlgAv0_bXYLyQP1r1AkMiJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udaHfJfYsNunc1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhg">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=wd_10min&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-wd_10min">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-wd_10min">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-wd_10min">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-wd_10min"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-wd_10min" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-wd_10min">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>112.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-zcViQXijsKICcMwK74mUMqObsN2JTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udYnLQ1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhgA--">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=rh&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-rh">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-rh">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-rh">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-rh"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-rh" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-rh">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>96.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-jxp.df19vcWprnEFbrF4xzG8.ySJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udYnTI1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhgA--">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=td&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-td">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-td">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-td">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-td"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-td" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-td">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>-1.8</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-bm.6UXUIV4bG915nLER8KdZQ4CKJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udZHK.x0Nbbp589O7PUy.OlY07DObW7rtn5qfTD00793OcJrceuHZp6eZO7Nvia3Pph6ad.6p54Za0N_DLuyYemFuc.m_llyceuXl5v6claYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08mh007ctPpl4TNDpp25bW_dlrGq1IYA">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=r_1h&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-r_1h">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-r_1h">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-r_1h">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-r_1h"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-r_1h" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-r_1h">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>NaN</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-tsEtBkN_Vnv9MZue7hJ1N8CvQx2JTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udaHLTfYsNunc1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhg">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=ri_10min&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-ri_10min">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-ri_10min">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-ri_10min">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-ri_10min"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-ri_10min" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-ri_10min">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>1.1</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-vjcP_zLyCXLDqAfL9idhzarM05yJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udaHPdv738Pfm1tunnz07s9TL46VjTsM5tbuu2fmp9MPTTv3c5wmtx64dmnp5k7s2.Jrc.mHpp37qnnhlrQ38Mu7Jh6YW5z6b.WXJx65eXm_pyVphZtul0634ueXl2w9NO_dzdOu.XD00ZeTp1007cvbDs65eGHTyaHTTty0.mXhM0Omnbltb92WsarUhg">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=snow_aws&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-snow_aws">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-snow_aws">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-snow_aws">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-snow_aws"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-snow_aws" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-snow_aws">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>28.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-7vG7T94bwBqGULtNLevtGfklVrSJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udZXC_zy4Wtt08.endnqZfHSsadhnNrd12z81Pph6ad.7nOE1uPXDs09PMndm3xNbn0w9NO_dU88MtaG_hl3ZMPTC3OfTfyy5OPXLy839OStMLNt0unW_Fzy8u2Hpp37ubp13y4emjLydOumnbl7YdnXLww6eTQ6aduWn0y8Jmh007ctrfuy1jVakM">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=p_sea&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-p_sea">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-p_sea">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-p_sea">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-p_sea"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-p_sea" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-p_sea">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>1018.7</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-N5xaqjVL44EZqTQxy7MaGBj.PpeJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udY3bTza23Tz56d2epl8dKxp2Gc2t3XbPzU.mHpp37uc4TW49cOzT08yd2bfE1ufTD00791Tzwy1ob.GXdkw9MLc59N_LLk49cvLzf05K0ws23S6db8XPLy7Yemnfu5unXfLh6aMvJ066aduXth2dcvDDp5NDpp25afTLwmaHTTty2t.7LWNVqQwA-">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=vis&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-vis">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-vis">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-vis">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-vis"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-vis" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-vis">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>3900.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-uX_r.J9oo16a26qswvUy1jlrjKqJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udZW6_tw7mtt08.endnqZfHSsadhnNrd12z81Pph6ad.7nOE1uPXDs09PMndm3xNbn0w9NO_dU88MtaG_hl3ZMPTC3OfTfyy5OPXLy839OStMLNt0unW_Fzy8u2Hpp37ubp13y4emjLydOumnbl7YdnXLww6eTQ6aduWn0y8Jmh007ctrfuy1jVakM">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=n_man&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-n_man">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-n_man">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-n_man">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-n_man"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-n_man" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-n_man">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>8.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
	    <wfs:member>
                <omso:PointTimeSeriesObservation gml:id="WFS-TADawYpRxxYZF6nEauL6nTstjhKJTowqYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08oWliy59O6pp25bUv8KJq0f5QmNj5c61ItCnHdOmjJq4Z2XdkqaduW1L_CiasGWcJnZtunnpyc6zGLBg5bsXRry.e._lkv7.2Xl35aemHFsyxMzZh6ZefSJmbN.PDsy1qZtN.NJXdemZw1tuHxE08.mHdjy0rV0IDS24fEXhvx6Oc4Mcze25emXfQw8sO3L0y8udZHfD3wtbbp589O7PUy.OlY07DObW7rtn5qfTD00793OcJrceuHZp6eZO7Nvia3Pph6ad.6p54Za0N_DLuyYemFuc.m_llyceuXl5v6claYWbbpdOt.Lnl5dsPTTv3c3Trvlw9NGXk6ddNO3L2w7OuXhh08mh007ctPpl4TNDpp25bW_dlrGq1IYA">

		      
      <om:phenomenonTime xlink:href="#time1-1-1"/>
      <om:resultTime xlink:href="#time2-1-1"/>      

		<om:procedure xlink:href="http://xml.fmi.fi/inspire/process/opendata"/>
   		            <om:parameter>
                <om:NamedValue>
                    <om:name xlink:href="http://inspire.ec.europa.eu/codeList/ProcessParameterValue/value/groundObservation/observationIntent"/>
                    <om:value>
			atmosphere
                    </om:value>
                </om:NamedValue>
            </om:parameter>

                <om:observedProperty xlink:href="https://opendata.fmi.fi/meta?observableProperty=observation&amp;param=wawa&amp;language=eng"/>
				<om:featureOfInterest>
                    <sams:SF_SpatialSamplingFeature gml:id="fi-1-1-wawa">
          <sam:sampledFeature>
		<target:LocationCollection gml:id="sampled-target-1-1-wawa">
		    <target:member>
		    <target:Location gml:id="obsloc-fmisid-100971-pos-wawa">
		        <gml:identifier codeSpace="http://xml.fmi.fi/namespace/stationcode/fmisid">100971</gml:identifier>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/name">Helsinki Kaisaniemi</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/geoid">-16000150</gml:name>
			<gml:name codeSpace="http://xml.fmi.fi/namespace/locationcode/wmo">2978</gml:name>
			<target:representativePoint xlink:href="#point-fmisid-100971-1-1-wawa"/>
			
			
			<target:region codeSpace="http://xml.fmi.fi/namespace/location/region">Helsinki</target:region>
			
		    </target:Location></target:member>
		</target:LocationCollection>
 	   </sam:sampledFeature>
                        <sams:shape>
                            
			    <gml:Point gml:id="point-fmisid-100971-1-1-wawa" srsName="http://www.opengis.net/def/crs/EPSG/0/4258" srsDimension="2">
                                <gml:name>Helsinki Kaisaniemi</gml:name>
                                <gml:pos>60.17523 24.94459 </gml:pos>
                            </gml:Point>
                            
                        </sams:shape>
                    </sams:SF_SpatialSamplingFeature>
                </om:featureOfInterest>

		  <om:result>
                    <wml2:MeasurementTimeseries gml:id="obs-obs-1-1-wawa">                         
                        <wml2:point>
                            <wml2:MeasurementTVP> 
                                      <wml2:time>2021-02-21T14:30:00Z</wml2:time>
				      <wml2:value>64.0</wml2:value>
                            </wml2:MeasurementTVP>
                        </wml2:point>                         
                    </wml2:MeasurementTimeseries>
                </om:result>

        </omso:PointTimeSeriesObservation>
    </wfs:member>
</wfs:FeatureCollection>"###;

    #[tokio::test]
    async fn fmi() {
        let parsed = parse_xml(&FMI_XML).unwrap();
        assert_eq!(parsed.place, Some("Helsinki Kaisaniemi".to_owned()));
        assert_eq!(parsed.temperature, Some("-1.3".to_owned()));
        assert_eq!(parsed.wind, Some("6.5".to_owned()));
        assert_eq!(parsed.gust, Some("9.0".to_owned()));
        assert_eq!(parsed.feels_like, Some("-5.9".to_owned()));
        assert_eq!(parsed.humidity, Some("96".to_owned()));
        assert_eq!(parsed.cloudiness, Some("8".to_owned()));
        assert_eq!(parsed.wawa, Some("jäätävää heikkoa vesisadetta".to_owned()));

        let msg = generate_msg(parsed);
        assert_eq!(msg, "Helsinki Kaisaniemi: lämpötila: -1.3°C, tuntuu kuin: -5.9°C, tuulen nopeus: 6.5m/s, puuskat: 9.0m/s, ilman kosteus: 96%, pilvisyys: 8/8, jäätävää heikkoa vesisadetta");
    }
}
