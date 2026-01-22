use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{json, Value};
use std::{env, fs};
use url::Url;

use crate::network::retry_request;
use crate::verification;

const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 12; SM-A5560 Build/V417IR; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/101.0.4951.61 Safari/537.36; SKLand/1.52.1";

pub fn get_tokens() -> Vec<String> {
    let tokens: Vec<String> = match env::var("USER_TOKENS") {
        Ok(val) => val.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Err(_) => {
            println!("The USER_TOKENS variable was not found in the environment variables, attempting to read from user_tokens.txt.");
            match fs::read_to_string("user_tokens.txt") {
                Ok(val) => val.split('\n').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                Err(_) => panic!("Unable to find USER_TOKENS environment variable or user_tokens.txt file!"),
            }
        }
    };
    if tokens.is_empty() {
        panic!("No user tokens found!");
    } else {
        println!("Got {} user tokens successfully!", tokens.len());
    }
    tokens
}

pub fn generate_headers(client: &Client) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
    headers.insert("Connection", HeaderValue::from_static("close"));
    headers.insert("X-Requested-With", HeaderValue::from_static("com.hypergryph.skland"));
    headers.insert("dId", HeaderValue::from_str(&verification::get_did(client)).unwrap());
    headers
}

pub fn get_authorization(client: &Client, headers: &HeaderMap, token: &str) -> String {
    let authorization_response: Value = retry_request(|| {
        let resp = client
            .post("https://as.hypergryph.com/user/oauth2/v2/grant")
            .headers(headers.clone())
            .json(&json!({"appCode": "4ca99fa6b56cc2ba", "token": token, "type": 0}))
            .send()?
            .json()?;
        Ok(resp)
    });
    if authorization_response["status"] != 0 {
        panic!("Failed to get authorization: {}", authorization_response["message"]);
    }
    authorization_response["data"]["code"].as_str().expect("Not a String!").to_string()
}

pub fn get_credential(client: &Client, headers: &HeaderMap, authorization: &str) -> Value {
    let credential_response: Value = retry_request(|| {
        let resp = client
            .post("https://zonai.skland.com/web/v1/user/auth/generate_cred_by_code")
            .headers(headers.clone())
            .json(&json!({"code": authorization, "kind": 1}))
            .send()?
            .json()?;
        Ok(resp)
    });
    if credential_response["code"] != 0 {
        panic!("Failed to get credential: {}", credential_response["message"]);
    }
    credential_response["data"].clone()
}

pub fn do_sign(cred_resp: &Value) {
    let http_token = cred_resp["token"].as_str().unwrap();
    let cred = cred_resp["cred"].as_str().unwrap();
    let client = Client::new();
    let mut http_header = HeaderMap::new();
    http_header.insert("User-Agent", HeaderValue::from_static(USER_AGENT));
    http_header.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
    http_header.insert("Connection", HeaderValue::from_static("close"));
    http_header.insert("X-Requested-With", HeaderValue::from_static("com.hypergryph.skland"));
    http_header.insert("cred", HeaderValue::from_str(cred).unwrap());
    http_header.insert("dId", HeaderValue::from_str(&verification::get_did(&client)).unwrap());
    let characters = get_binding_list(&http_header, http_token);
    for character in characters {
        let app_code = character["appCode"].as_str().unwrap_or("arknights");
        let game_name = character["gameName"].as_str().unwrap_or("Unknown");
        let nick_name = character["nickName"].as_str().unwrap_or("Unknown");
        let channel_name = character["channelName"].as_str().unwrap_or("Unknown");

        match app_code {
            "arknights" => sign_for_arknights(&client, &http_header, http_token, &character, game_name, nick_name, channel_name),
            "endfield" => sign_for_endfield(&client, &http_header, http_token, &character, game_name, nick_name, channel_name),
            _ => {}
        }
    }
}

fn sign_for_arknights(client: &Client, http_header: &HeaderMap, http_token: &str, character: &Value, game_name: &str, nick_name: &str, channel_name: &str) {
    let body = json!({"gameId": character["gameId"].as_i64().unwrap_or(1), "uid": character["uid"].as_str().unwrap()});
    let url = "https://zonai.skland.com/api/v1/game/attendance";
    let headers = get_sign_header(url, "post", Some(body.to_string().as_str()), http_header, http_token);
    let response: Value = retry_request(|| {
        let resp = client.post(url).headers(headers.clone()).json(&body).send()?.json()?;
        Ok(resp)
    });
    if response["code"].as_i64().unwrap() != 0 {
        eprintln!(
            "[{}]{}({}) sign-in failed! Reason: {}",
            game_name, nick_name, channel_name,
            response["message"].as_str().unwrap_or("Unknown error")
        );
        return;
    }
    for award in response["data"]["awards"].as_array().unwrap() {
        let name = award["resource"]["name"].as_str().unwrap_or("Unknown");
        let count = award["count"].as_i64().unwrap_or(1);
        println!("[{}]{}({}) signed in successfully and received 「{}」×{}.", game_name, nick_name, channel_name, name, count);
    }
}

fn sign_for_endfield(client: &Client, http_header: &HeaderMap, http_token: &str, character: &Value, game_name: &str, nick_name: &str, channel_name: &str) {
    let roles = match character["roles"].as_array() {
        Some(r) => r.clone(),
        None => {
            eprintln!("[{}]{}({}) has no roles!", game_name, nick_name, channel_name);
            return;
        }
    };
    for role in roles {
        let role_nickname = role["nickname"].as_str().unwrap_or(nick_name);
        let role_id = role["roleId"].as_str().unwrap_or("");
        let server_id = role["serverId"].as_str().unwrap_or("");
        let url = "https://zonai.skland.com/web/v1/game/endfield/attendance";
        let mut headers = get_sign_header(url, "post", Some(""), http_header, http_token);
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("sk-game-role", HeaderValue::from_str(&format!("3_{}_{}", role_id, server_id)).unwrap());
        headers.insert("referer", HeaderValue::from_static("https://game.skland.com/"));
        headers.insert("origin", HeaderValue::from_static("https://game.skland.com/"));
        let response: Value = retry_request(|| {
            let resp = client.post(url).headers(headers.clone()).send()?.json()?;
            Ok(resp)
        });
        if response["code"].as_str().unwrap_or("") != "10000" {
            eprintln!(
                "[{}]{}({}) sign-in failed! Reason: {}",
                game_name, role_nickname, channel_name,
                response["message"].as_str().unwrap_or("Unknown error")
            );
            continue;
        }
        println!("[{}]{}({}) signed in successfully! Response: {}", game_name, role_nickname, channel_name, response);
    }
}

fn get_binding_list(http_header: &HeaderMap, http_token: &str) -> Vec<Value> {
    let client = reqwest::blocking::Client::new();
    let sign_header = get_sign_header("https://zonai.skland.com/api/v1/game/player/binding", "get", None, http_header, http_token);
    let resp: Value = retry_request(|| {
        let resp = client.get("https://zonai.skland.com/api/v1/game/player/binding").headers(sign_header.clone()).send()?.json()?;
        Ok(resp)
    });
    if resp["code"] != 0 {
        eprintln!("An issue occurred while requesting the character list.: {}", resp["message"]);
        if resp["message"] == "用户未登录" {
            eprintln!("User login may have expired. Please rerun this program!");
        }
        return vec![];
    }
    resp["data"]["list"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| {
            let app_code = i["appCode"].as_str()?;
            if app_code != "arknights" && app_code != "endfield" {
                return None;
            }
            Some(i["bindingList"].as_array()?.iter().map(|binding| {
                let mut b = binding.clone();
                b["appCode"] = json!(app_code);
                b
            }).collect::<Vec<_>>())
        })
        .flatten()
        .collect()
}

fn get_sign_header(url: &str, method: &str, body: Option<&str>, header: &HeaderMap, token: &str) -> HeaderMap {
    let parsed_url = Url::parse(url).expect("Invalid URL");
    let did = header.get("dId").and_then(|v| v.to_str().ok()).unwrap_or("");
    let (sign, header_ca) = match method.to_lowercase().as_str() {
        "get" => verification::generate_signature(token, parsed_url.path(), parsed_url.query().unwrap_or(""), did),
        _ => verification::generate_signature(token, parsed_url.path(), body.unwrap_or(""), did),
    };
    let mut header_clone = header.clone();
    header_clone.insert("sign", sign.parse().unwrap());
    for (key, value) in header_ca {
        header_clone.insert(
            HeaderName::from_bytes(key.as_bytes()).unwrap(),
            match value {
                Value::Number(num) => HeaderValue::from_str(&num.to_string()).unwrap(),
                Value::String(s) => HeaderValue::from_str(&s).unwrap(),
                _ => panic!("Unexpected value type: {:?}", value),
            },
        );
    }
    header_clone
}
