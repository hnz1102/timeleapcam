use config::{File, FileFormat, Config as NvsConfig};
use std::collections::HashMap;
use log::info;

#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_psk: &'static str,
    #[default("0")]
    timezone_offset: &'static str,  // Timezone offset from UTC -12 to +14
    #[default("300")]
    idle_in_sleep_time: &'static str,   // 0: disable sleep, 1-: sleep time in seconds when no key input
    #[default("false")]
    auto_capture: &'static str,   // true: auto capture, false: manual capture
    #[default("0")]
    resolution: &'static str,   // 0: 640x480, 1: 800x600, 2: 1024x768, 3: 1280x1024, 4: 1600x1200, 5: 2048x1536
    #[default("0")]
    track_id: &'static str,
    #[default("0")]
    duration: &'static str,   // 0: no limit, 1-: duration in seconds
    #[default("gpt-4-turbo")]
    model: &'static str,
    #[default("api-key")]
    api_key: &'static str,
    #[default("false")]
    query_openai: &'static str,
    #[default("")]
    query_prompt: &'static str,
    #[default("")]
    post_account: &'static str,
    #[default("")]
    post_access_token: &'static str,
    #[default("")]
    storage_account: &'static str,
    #[default("")]
    storage_access_token: &'static str,
    #[default("")]
    post_message_trigger: &'static str,
}

const MENU_SSID: (&str, &str) = ("SSID", "ssid");
const MENU_PSK: (&str, &str) = ("PSK", "psk");
const MENU_TIMEZONE: (&str, &str) = ("TIMEZONE", "timezone");
const MENU_IDLESLEEP: (&str, &str) = ("IDLESLEEP", "idlesleep");
const MENU_AUTOCAPTURE: (&str, &str) = ("AUTOCAPTURE", "autocapture");
const MENU_RESOLUTION: (&str, &str) = ("RESOLUTION", "resolution");
const MENU_TRACKID: (&str, &str) = ("TRACKID", "trackid");
const MENU_DURATION: (&str, &str) = ("DURATION", "duration");
const MENU_MODEL: (&str, &str) = ("MODEL", "model");
const MENU_APIKEY: (&str, &str) = ("APIKEY", "apikey");
const MENU_QUERYOPENAI: (&str, &str) = ("QUERYOPENAI", "queryopenai");
const MENU_QUERYPROMPT: (&str, &str) = ("QUERYPROMPT", "queryprompt");
const MENU_POSTACCOUNT: (&str, &str) = ("POSTACCOUNT", "postaccount");
const MENU_POSTACCESSTOKEN: (&str, &str) = ("POSTACCESSTOKEN", "postaccesstoken");
const MENU_STORAGEACCOUNT: (&str, &str) = ("STORAGEACCOUNT", "storageaccount");
const MENU_STORAGEACCESSTOKEN: (&str, &str) = ("STORAGEACCESSTOKEN", "storageaccesstoken");
const MENU_POSTMESSAGETRIGGER: (&str, &str) = ("POSTMESSAGETRIGGER", "postmessagetrigger");

#[derive(Debug)]
pub struct ConfigData {
    pub wifi_ssid: String,
    pub wifi_psk: String,
    pub timezone_offset: i32,
    pub idle_in_sleep_time: u32,
    pub auto_capture: bool,
    pub resolution: u32,
    pub track_id: u32,
    pub duration: u32,
    pub model: String,
    pub api_key: String,
    pub query_openai: bool,
    pub query_prompt: String,
    pub post_account: String,
    pub post_access_token: String,
    pub storage_account: String,
    pub storage_access_token: String,
    pub post_message_trigger: String,
}

impl ConfigData {
    pub fn new() -> ConfigData {
        ConfigData {
            wifi_ssid: String::new(),
            wifi_psk: String::new(),
            timezone_offset: 0,
            idle_in_sleep_time: 0,
            auto_capture: false,
            resolution: 0,
            track_id: 0,
            duration: 0,
            model: String::new(),
            api_key: String::new(),
            query_openai: false,
            query_prompt: String::new(),
            post_account: String::new(),
            post_access_token: String::new(),
            storage_account: String::new(),
            storage_access_token: String::new(),
            post_message_trigger: String::new(),
        }
    }
    pub fn load_config(&mut self, nvs_value: Option<&str>) -> anyhow::Result<()> {
        if nvs_value == None {
            return Err(anyhow::Error::msg("nvs_value is None"));
        }
        let settings = NvsConfig::builder()
        .add_source(File::from_str(&nvs_value.unwrap(), FileFormat::Toml))
        .build()?;
        let settings_map = settings.try_deserialize::<HashMap<String, String>>()?;
        info!("settings_map: {:?}", settings_map);
        self.wifi_ssid = settings_map.get(MENU_SSID.1).ok_or(anyhow::Error::msg("wifi_ssid not found"))?.to_string();
        self.wifi_psk = settings_map.get(MENU_PSK.1).ok_or(anyhow::Error::msg("wifi_psk not found"))?.to_string();
        self.timezone_offset = settings_map.get(MENU_TIMEZONE.1).ok_or(anyhow::Error::msg("timezone_offset not found"))?.parse::<i32>()?;
        self.idle_in_sleep_time = settings_map.get(MENU_IDLESLEEP.1).ok_or(anyhow::Error::msg("idle_in_sleep_time not found"))?.parse::<u32>()?;
        self.auto_capture = settings_map.get(MENU_AUTOCAPTURE.1).ok_or(anyhow::Error::msg("auto_capture not found"))?.parse::<bool>()?;
        self.resolution = settings_map.get(MENU_RESOLUTION.1).ok_or(anyhow::Error::msg("resolution not found"))?.parse::<u32>()?;
        self.track_id = settings_map.get(MENU_TRACKID.1).ok_or(anyhow::Error::msg("track_id not found"))?.parse::<u32>()?;
        self.duration = settings_map.get(MENU_DURATION.1).ok_or(anyhow::Error::msg("duration not found"))?.parse::<u32>()?;
        self.model = settings_map.get(MENU_MODEL.1).ok_or(anyhow::Error::msg("model not found"))?.to_string();
        self.api_key = settings_map.get(MENU_APIKEY.1).ok_or(anyhow::Error::msg("api_key not found"))?.to_string();
        self.query_openai = settings_map.get(MENU_QUERYOPENAI.1).ok_or(anyhow::Error::msg("query_openai not found"))?.parse::<bool>()?;
        self.query_prompt = settings_map.get(MENU_QUERYPROMPT.1).ok_or(anyhow::Error::msg("query_prompt not found"))?.to_string();
        self.post_account = settings_map.get(MENU_POSTACCOUNT.1).ok_or(anyhow::Error::msg("post_account not found"))?.to_string();
        self.post_access_token = settings_map.get(MENU_POSTACCESSTOKEN.1).ok_or(anyhow::Error::msg("post_access_token not found"))?.to_string();
        self.storage_account = settings_map.get(MENU_STORAGEACCOUNT.1).ok_or(anyhow::Error::msg("storage_account not found"))?.to_string();
        self.storage_access_token = settings_map.get(MENU_STORAGEACCESSTOKEN.1).ok_or(anyhow::Error::msg("storage_access_token not found"))?.to_string();
        self.post_message_trigger = settings_map.get(MENU_POSTMESSAGETRIGGER.1).ok_or(anyhow::Error::msg("post_message_trigger not found"))?.to_string();
        Ok(())
    }
    
    pub fn set_default_config(&self) -> Vec::<(String, String)> {
        let mut default_config = Vec::<(String, String)>::new();
        default_config.push((MENU_SSID.0.to_string(), CONFIG.wifi_ssid.to_string()));
        default_config.push((MENU_PSK.0.to_string(),  CONFIG.wifi_psk.to_string()));
        default_config.push((MENU_TIMEZONE.0.to_string(), CONFIG.timezone_offset.to_string()));
        default_config.push((MENU_IDLESLEEP.0.to_string(), CONFIG.idle_in_sleep_time.to_string()));
        default_config.push((MENU_AUTOCAPTURE.0.to_string(), CONFIG.auto_capture.to_string()));
        default_config.push((MENU_RESOLUTION.0.to_string(), CONFIG.resolution.to_string()));
        default_config.push((MENU_TRACKID.0.to_string(), CONFIG.track_id.to_string()));
        default_config.push((MENU_DURATION.0.to_string(), CONFIG.duration.to_string()));
        default_config.push((MENU_MODEL.0.to_string(), CONFIG.model.to_string()));
        default_config.push((MENU_APIKEY.0.to_string(), CONFIG.api_key.to_string()));
        default_config.push((MENU_QUERYOPENAI.0.to_string(), CONFIG.query_openai.to_string()));
        default_config.push((MENU_QUERYPROMPT.0.to_string(), CONFIG.query_prompt.to_string()));
        default_config.push((MENU_POSTACCOUNT.0.to_string(), CONFIG.post_account.to_string()));
        default_config.push((MENU_POSTACCESSTOKEN.0.to_string(), CONFIG.post_access_token.to_string()));
        default_config.push((MENU_STORAGEACCOUNT.0.to_string(), CONFIG.storage_account.to_string()));
        default_config.push((MENU_STORAGEACCESSTOKEN.0.to_string(), CONFIG.storage_access_token.to_string()));
        default_config.push((MENU_POSTMESSAGETRIGGER.0.to_string(), CONFIG.post_message_trigger.to_string()));
        default_config
    }

    #[allow(dead_code)]
    pub fn get_all_config(&self) -> Vec::<(String, String)> {
        let mut all_config = Vec::<(String, String)>::new();
        all_config.push((MENU_SSID.0.to_string(), self.wifi_ssid.to_string()));
        all_config.push((MENU_PSK.0.to_string(),  self.wifi_psk.to_string()));
        all_config.push((MENU_TIMEZONE.0.to_string(), self.timezone_offset.to_string()));
        all_config.push((MENU_IDLESLEEP.0.to_string(), self.idle_in_sleep_time.to_string()));
        all_config.push((MENU_AUTOCAPTURE.0.to_string(), self.auto_capture.to_string()));
        all_config.push((MENU_RESOLUTION.0.to_string(), self.resolution.to_string()));
        all_config.push((MENU_TRACKID.0.to_string(), self.track_id.to_string()));
        all_config.push((MENU_DURATION.0.to_string(), self.duration.to_string()));
        all_config.push((MENU_MODEL.0.to_string(), self.model.to_string()));
        all_config.push((MENU_APIKEY.0.to_string(), self.api_key.to_string()));
        all_config.push((MENU_QUERYOPENAI.0.to_string(), self.query_openai.to_string()));
        all_config.push((MENU_QUERYPROMPT.0.to_string(), self.query_prompt.to_string()));
        all_config.push((MENU_POSTACCOUNT.0.to_string(), self.post_account.to_string()));
        all_config.push((MENU_POSTACCESSTOKEN.0.to_string(), self.post_access_token.to_string()));
        all_config.push((MENU_STORAGEACCOUNT.0.to_string(), self.storage_account.to_string()));
        all_config.push((MENU_STORAGEACCESSTOKEN.0.to_string(), self.storage_access_token.to_string()));
        all_config.push((MENU_POSTMESSAGETRIGGER.0.to_string(), self.post_message_trigger.to_string()));
        all_config
    }    
}

