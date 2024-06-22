use log::*;
use embedded_svc::http::client::Client;
use esp_idf_svc::http::client::{EspHttpConnection, Configuration};
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;
use embedded_svc::http::Method;
use esp_idf_hal::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::path::Path;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use url::Url;

type HmacSha256 = Hmac<Sha256>;
const EXPIRATION: u64 = 60 * 60 * 24; // 1 day

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
    storage_signed_key: String,
    image_url: String,
    post_message_string: String,
    track_id: u32,
    count: u32,
    posted_status: bool,
    last_posted_date_time: SystemTime,
    post_interval: u32,
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
                storage_signed_key: String::from(""),
                image_url: String::from(""),
                post_message_string: String::from(""),
                track_id: 0,
                count: 0,
                posted_status: false,
                last_posted_date_time: SystemTime::now(),
                post_interval: 0,
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
                    if postmsg.post_interval > 0 && postmsg.last_posted_date_time.elapsed().unwrap().as_secs() < postmsg.post_interval as u64 {
                        info!("Post interval not reached");
                    }
                    else {
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
                                    postmsg.storage_access_token.clone(), filename, &buffer, true);
                            post_image_status = match image_url {
                                Ok(url) => {
                                    let signed_url = generate_signed_url(Url::parse(&url).unwrap(), postmsg.storage_signed_key.as_str());
                                    info!("Signed URL: {:?}", signed_url);
                                    postmsg.image_url = signed_url;
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
                                    postmsg.posted_status = true;
                                    info!("Message posted successfully");
                                }
                                Err(e) => {
                                    info!("Failed to post message: {:?}", e);
                                }
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
                            postmsg.storage_access_token.clone(), filename, &buffer, true);
                    let post_image_status = match image_url {
                        Ok(url) => {
                            let signed_url = generate_signed_url(Url::parse(&url).unwrap(), postmsg.storage_signed_key.as_str());
                            info!("Signed URL: {:?}", signed_url);
                            postmsg.image_url = signed_url;
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
                    let image_list = delete_images(postmsg.storage_url.clone(), postmsg.storage_account.clone(),
                        postmsg.storage_access_token.clone());
                    match image_list {
                        Ok(list) => {
                            for image in list.as_array().unwrap() {
                                let image_id = image["id"].as_str().unwrap();
                                let upload_date = image["uploaded"].as_str().unwrap();
                                info!("Image ID: {:?} Uploaded: {:?}", image_id, upload_date);
                                // parse upload date <2024-06-21T12:23:13.576Z> to seconds
                                let upload_date_sec_utc = upload_date.parse::<chrono::DateTime<chrono::Utc>>().unwrap().timestamp();
                                // upload date is older than EXPIRATION
                                if SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - (upload_date_sec_utc as u64) < EXPIRATION {
                                    continue;
                                }
                                let _result = delete_image(postmsg.storage_url.clone(), postmsg.storage_account.clone(),
                                    postmsg.storage_access_token.clone(), image_id.to_string());
                            }
                        }
                        Err(e) => {
                            info!("Failed to delete images: {:?}", e);
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

    pub fn set_storage_access_token(&self, account: String, access_token: String, signed_key: String) {
        let mut postmsg = self.postmsg.lock().unwrap();
        postmsg.storage_account = account;
        postmsg.storage_access_token = access_token;
        postmsg.storage_signed_key = signed_key;
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

    pub fn get_posted_status(&self) -> bool {
        let postmsg = self.postmsg.lock().unwrap();
        postmsg.posted_status
    }

    pub fn set_last_posted_date_time(&self, datetime: SystemTime, post_interval: u32) {
        let mut postmsg = self.postmsg.lock().unwrap();
        postmsg.last_posted_date_time = datetime;
        postmsg.post_interval = post_interval;
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
              filename: String, buffer: &Vec<u8>, signed_url: bool) -> anyhow::Result<String> {
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
    request.write(b"\r\n--------------------------a90531928d528eaf\r\n")?;
    let signed_url_form = format!("Content-Disposition: form-data; name=\"requireSignedURLs\"\r\n\r\n{}\r\n",
        if signed_url {"true"} else {"false"});
    request.write(signed_url_form.as_bytes())?;
    request.write(b"--------------------------a90531928d528eaf--\r\n")?;
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


// Generate signed URL
fn generate_signed_url(mut url: Url, key: &str) -> String {
    // Get current UNIX timestamp and add expiration
    info!("URL: {:?} key:{:?}", url, key);
    let expiry = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + EXPIRATION;
    url.query_pairs_mut().append_pair("exp", &expiry.to_string());

    // Create a string to sign
    let string_to_sign = format!("{}?{}", url.path(), url.query().unwrap_or_default());

    // Generate a signature using HMAC-SHA256
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    let signature = mac.finalize().into_bytes();

    // Convert the signature to a hexadecimal string and add it to the URL
    url.query_pairs_mut().append_pair("sig", &hex::encode(signature));

    url.to_string()
}


fn delete_images(storage_url: String, storage_account: String, storage_access_token: String)
         -> anyhow::Result<serde_json::Value> {
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
        ("Content-Type", "application/json"),
    ];
    let url = format!("{}/{}/images/v1", storage_url, storage_account);
    let mut request = client.request(Method::Get, 
        &url,
        &headers)?;
    // get image list
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    info!("Get Image List Status: {:?}", status);
    let mut buf = Vec::new();
    let mut tmpbuf = [0u8; 8192];
    let mut image_list : serde_json::Value = serde_json::Value::Null;
    match status {
        200 => {
            loop {
                let len = response.read(&mut tmpbuf)?;
                if len == 0 {
                    break;
                }
                buf.extend_from_slice(&tmpbuf[..len]);
            }
            let body_length = buf.len();
            info!("Body Length: {:?}", body_length);
            let json : serde_json::Value = serde_json::from_slice(&buf[..body_length]).unwrap();
            image_list = json["result"]["images"].clone();
        }
        _ => {
            let len = response.read(&mut buf)?;
            info!("Response Error {} {:?}", status, std::str::from_utf8(&buf[..len]));
        }
    }
    Ok(image_list)
}

fn delete_image(storage_url: String, storage_account: String, storage_access_token: String,
                image_id: String) -> anyhow::Result<()> {
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
        ("Content-Type", "application/json"),
    ];
    let url = format!("{}/{}/images/v1/{}", storage_url, storage_account, image_id);
    let mut request = client.request(Method::Delete, 
        &url,
        &headers)?;
    request.flush()?;
    let mut response = request.submit()?;
    let status = response.status();
    info!("Delete Image url:{:?} status:{:?}", url, status);
    let mut buf = [0u8; 1024];
    match status {
        200 => {
            info!("Image deleted successfully");
        }
        _ => {
            let len = response.read(&mut buf)?;
            info!("Response Error {} {:?}", status, std::str::from_utf8(&buf[..len]));
        }
    }
    Ok(())
}
