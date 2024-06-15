use log::*;
use embedded_svc::http::client::Client;
use esp_idf_svc::http::client::{EspHttpConnection, Configuration};
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;
use embedded_svc::http::Method;
use esp_idf_hal::io::Write;
use std::time::Duration;
use std::path::Path;

use crate::imagefiles;

struct QueryOpenAI {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub query_start: bool,
    pub detail: String,
    pub reply: String,
    pub track_id: u32,
    pub count: u32,
    pub prompt: String,
    pub timeout: u32,
}

struct PostImageAndMessage {
    post_message_request: bool,
    post_url: String,
    post_to: String,
    post_message_trigger: String,
    access_token: String,
    storage_url: String,
    storage_account: String,
    storage_access_token: String,
    image_url: String,
    post_message_string: String,
    track_id: u32,
    count: u32,
}

pub struct Monitoring {
    openai: Arc<Mutex<QueryOpenAI>>,
    postmsg: Arc<Mutex<PostImageAndMessage>>,
}

impl Monitoring {
    pub fn new(model: String, api_key: String) -> Self {
        Monitoring {
            openai: Arc::new(Mutex::new(QueryOpenAI {
                api_key,
                model,
                max_tokens: 100,
                reply: String::from(""),
                query_start: false,
                detail: String::from("low"),
                track_id: 0,
                count: 0,
                prompt: String::from(""),
                timeout: 20,
            })),
            postmsg: Arc::new(Mutex::new(PostImageAndMessage {
                post_message_request: false,
                post_url: String::from("https://api.line.me/v2/bot/message/push"),
                post_to: String::from(""),
                post_message_trigger: String::from(""),
                access_token: String::from(""),
                storage_url: String::from("https://api.cloudflare.com/client/v4/accounts"),
                storage_account: String::from(""),
                storage_access_token: String::from(""),
                image_url: String::from(""),
                post_message_string: String::from(""),
                track_id: 0,
                count: 0,
            })),
        }
    }

    // Query thread to OpenAI using EspHttpConnection
    pub fn start(&self) {
        let openai_info = self.openai.clone();
        let post_message_info = self.postmsg.clone();
        thread::spawn(move || {
            info!("Query thread started");
            loop {
                let mut openai = openai_info.lock().unwrap();
                let mut postmsg = post_message_info.lock().unwrap();
                if openai.query_start {
                    let file_path = format!("/eMMC/T{}/I{}.jpg", openai.track_id, openai.count);
                    let buffer = imagefiles::read_file(Path::new(&file_path));
                    let result = query_to_openai(openai.api_key.clone(), openai.model.clone(),
                        openai.prompt.clone(), openai.detail.clone(), openai.max_tokens,
                        openai.timeout, &buffer);
                    let openai_status = match result {
                        Ok(reply) => {
                            openai.reply = reply;
                            true
                        }
                        Err(_e) => {
                            false
                        }
                    };
                    // find string in reply
                    let port_trigger = postmsg.post_message_trigger.clone();
                    let found = match openai.reply.find(port_trigger.as_str()) {
                        Some(_) => true,
                        None => {
                            info!("No {} found in reply.", port_trigger.as_str());
                            false
                        }
                    };
                    let mut post_image_status = false;
                    if openai_status && found {
                        let filename = format!("t{}i{}.jpg", openai.track_id, openai.count);
                        let image_url = post_image(postmsg.storage_url.clone(),
                                postmsg.storage_account.clone(),
                                postmsg.storage_access_token.clone(), filename, &buffer);
                        post_image_status = match image_url {
                            Ok(url) => {
                                postmsg.image_url = url;
                                true
                            }
                            Err(e) => {
                                info!("Failed to post image: {:?}", e);
                                false
                            }
                        };
                    }
                    if post_image_status && found {
                        let result = post_message(postmsg.post_url.clone(), postmsg.post_to.clone(),
                            postmsg.access_token.clone(), postmsg.image_url.clone(), openai.reply.clone());
                        match result {
                            Ok(_) => {
                                info!("Message posted successfully");
                            }
                            Err(e) => {
                                info!("Failed to post message: {:?}", e);
                            }
                        }
                    }
                    openai.query_start = false;
                }
                if postmsg.post_message_request {
                    let file_path = format!("/eMMC/T{}/I{}.jpg", postmsg.track_id, postmsg.count);
                    let buffer = imagefiles::read_file(Path::new(&file_path));
                    let filename = format!("t{}i{}.jpg", postmsg.track_id, postmsg.count);
                    let image_url = post_image(postmsg.storage_url.clone(),
                            postmsg.storage_account.clone(),
                            postmsg.storage_access_token.clone(), filename, &buffer);
                    let post_image_status = match image_url {
                        Ok(url) => {
                            postmsg.image_url = url;
                            true
                        }
                        Err(e) => {
                            info!("Failed to post image: {:?}", e);
                            false
                        }
                    };
                    if post_image_status {
                        let result = post_message(postmsg.post_url.clone(), postmsg.post_to.clone(),
                            postmsg.access_token.clone(), postmsg.image_url.clone(), postmsg.post_message_string.clone());
                        match result {
                            Ok(_) => {
                                info!("Message posted successfully");
                            }
                            Err(e) => {
                                info!("Failed to post message: {:?}", e);
                            }
                        }
                    }
                    postmsg.post_message_request = false;
                }
                drop(postmsg);
                drop(openai);
                thread::sleep(Duration::from_millis(100));
            }
        });
    }

    pub fn set_query_start(&self, prompt: String, track_id: u32, count: u32) {
        let mut openai = self.openai.lock().unwrap();
        openai.track_id = track_id;
        openai.count = count;
        openai.prompt = prompt;
        openai.query_start = true;
    }

    pub fn get_query_reply(&self) -> String {
        let openai = self.openai.lock().unwrap();
        openai.reply.clone()
    }

    pub fn get_query_status(&self) -> bool {
        let openai = self.openai.lock().unwrap();
        openai.query_start
    }

    pub fn set_post_access_token(&self, to: String, access_token: String, post_message_trigger: String) {
        let mut postmsg = self.postmsg.lock().unwrap();
        postmsg.post_to = to;
        postmsg.access_token = access_token;
        postmsg.post_message_trigger = post_message_trigger;
    }

    pub fn set_storage_access_token(&self, account: String, access_token: String) {
        let mut postmsg = self.postmsg.lock().unwrap();
        postmsg.storage_account = account;
        postmsg.storage_access_token = access_token;
    }

    pub fn post_message_request(&self, message: String, track_id: u32, count: u32) {
        let mut postmsg = self.postmsg.lock().unwrap();
        postmsg.post_message_request = true;
        postmsg.post_message_string = message;
        postmsg.track_id = track_id;
        postmsg.count = count;
    }

    pub fn get_post_message_status(&self) -> bool {
        let postmsg = self.postmsg.lock().unwrap();
        postmsg.post_message_request
    }
}

fn query_to_openai(api_key: String, model: String, prompt: String,
                 detail: String, max_tokens: u32,
                 timeout: u32, buffer: &Vec<u8>) -> anyhow::Result<String> {    
    let base64_image = base64::encode(buffer);
    let jsonstr = format!(r#"{{"model": "{}", "messages": [
{{"role": "user", "content": [
    {{"type": "text", "text": "{}"}},
    {{"type": "image_url", "image_url": {{"url": "data:image/jpeg;base64,{}", "detail": "{}" }}}}
]}}],
"max_tokens": {}}}"#, model, prompt, base64_image, detail, max_tokens);
    info!("Prompt: {:?}", prompt);
    let http = EspHttpConnection::new(
        &Configuration {
            use_global_ca_store: true,
            crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
            timeout: Some(Duration::from_secs(timeout as u64)),
            ..Default::default()
        })?;
    let mut client = Client::wrap(http);
    let authorization = &format!("Bearer {}", api_key);
    let headers : [(&str, &str); 2] = [
        ("Authorization", authorization),
        ("Content-Type", "application/json"),
    ];
    let mut request = client.request(Method::Post, 
        "https://api.openai.com/v1/chat/completions",
        &headers)?;
    let body = jsonstr.as_bytes();
    request.write_all(body)?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    info!("OpenAI Query Status: {:?}", status);
    let mut buf = [0u8; 4096];
    let mut reply : String = String::new();
    match status {
        200 => {
            let len = response.read(&mut buf)?;
            let json: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
            reply = json["choices"][0]["message"]["content"].as_str().unwrap().to_string();
        }
        _ => {
            info!("Error: {:?}", status);
        }
    }
    Ok(reply)
}

fn post_image(storage_url: String, storage_account: String, storage_access_token: String,
              filename: String, buffer: &Vec<u8>) -> anyhow::Result<String> {
    let http = EspHttpConnection::new(
        &Configuration {
            use_global_ca_store: true,
            crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
            timeout: Some(Duration::from_secs(20)),
            ..Default::default()
        })?;
    let mut client = Client::wrap(http);
    let authorization = &format!("Bearer {}", storage_access_token);
    let headers : [(&str, &str); 2] = [
        ("Authorization", authorization),
        ("Content-Type", "multipart/form-data; boundary=------------------------a90531928d528eaf"),
    ];
    let url = format!("{}/{}/images/v1", storage_url, storage_account);
    // info!("URL: {:?}", url);
    // info!("Request: {:?}", headers);
    let mut request = client.request(Method::Post, 
        &url,
        &headers)?;
    request.write(b"--------------------------a90531928d528eaf\r\n")?;
    let disposition = format!("Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n", filename);
    request.write(disposition.as_bytes())?;
    request.write(b"Content-Type: image/jpeg\r\n")?;
    request.write(b"\r\n")?;
    request.write(buffer)?;
    request.write(b"\r\n--------------------------a90531928d528eaf--\r\n")?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    info!("Post Image Status: {:?}", status);
    let mut buf = [0u8; 1024];
    let mut image_url : String = String::new();
    match status {
        200 => {
            let len = response.read(&mut buf)?;
            let json: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
            image_url = json["result"]["variants"][0].as_str().unwrap().to_string();
        }
        _ => {
            let len = response.read(&mut buf)?;
            info!("Response Error {} {:?}", status, std::str::from_utf8(&buf[..len]));
        }
    }
    Ok(image_url)
}

fn post_message(post_url: String, post_to: String, access_token: String, image_url: String, reply: String) -> anyhow::Result<()> {
    // remove carriage return and line feed
    let reply = reply.replace("\r", "").replace("\n", " ");
    let http = EspHttpConnection::new(
        &Configuration {
            use_global_ca_store: true,
            crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
            timeout: Some(Duration::from_secs(20)),
            ..Default::default()
        })?;
    let mut client = Client::wrap(http);
    let authorization = &format!("Bearer {}", access_token);
    let headers : [(&str, &str); 2] = [
        ("Authorization", authorization),
        ("Content-Type", "application/json"),
    ];
    let jsonstr = format!(r#"{{"to": "{}", "messages": [
{{"type": "text", "text": "{}"}},
{{"type": "image", "originalContentUrl": "{}", "previewImageUrl": "{}"}}
]}}"#, post_to, reply, image_url, image_url);
    // info!("Request: {:?}", jsonstr);
    // info!("Request headers: {:?}", headers);
    // info!("Post URL: {:?}", post_url); 
    let mut request = client.request(Method::Post, 
        &post_url,
        &headers)?;
    let body = jsonstr.as_bytes();
    request.write_all(body)?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    info!("Status: {:?}", status);
    let mut buf = [0u8; 1024];
    match status {
        200 => {
            info!("Message posted successfully");
            // let len = response.read(&mut buf)?;
            // let json: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
            // info!("Response: {:?}", json);
        }
        _ => {
            let len = response.read(&mut buf)?;
            info!("Response Error {} {:?}", status, std::str::from_utf8(&buf[..len]));
        }
    }
    Ok(())
}