use crate::net;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const LOGIN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const SCOPE: &str = "XboxLive.signin offline_access";

pub struct Session {
    pub name: String,
    pub uuid: String,
    pub token: String,
    pub user_type: &'static str,
}

pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub fn device_code(client_id: &str) -> Result<DeviceCode> {
    let (st, v) = net::post_form(&format!("{LOGIN}/devicecode"), &[("client_id", client_id), ("scope", SCOPE)])?;
    if st != 200 {
        bail!("Device-Code-Anfrage fehlgeschlagen: {}", v["error_description"].as_str().unwrap_or("unbekannt"));
    }
    Ok(DeviceCode {
        device_code: v["device_code"].as_str().unwrap_or_default().into(),
        user_code: v["user_code"].as_str().unwrap_or_default().into(),
        verification_uri: v["verification_uri"].as_str().unwrap_or("https://microsoft.com/link").into(),
        interval: v["interval"].as_u64().unwrap_or(5),
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
    })
}

/// Returns (access_token, refresh_token) once the user has confirmed the code.
pub fn poll_token(client_id: &str, dc: &DeviceCode, cancel: &AtomicBool) -> Result<(String, String)> {
    let start = Instant::now();
    let mut interval = dc.interval.max(1);
    loop {
        std::thread::sleep(Duration::from_secs(interval));
        if cancel.load(Ordering::SeqCst) {
            bail!("Anmeldung abgebrochen");
        }
        if start.elapsed().as_secs() > dc.expires_in {
            bail!("Der Code ist abgelaufen");
        }
        let (st, v) = net::post_form(
            &format!("{LOGIN}/token"),
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", &dc.device_code),
            ],
        )?;
        if st == 200 {
            return Ok((
                v["access_token"].as_str().unwrap_or_default().into(),
                v["refresh_token"].as_str().unwrap_or_default().into(),
            ));
        }
        match v["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            _ => bail!("Anmeldung fehlgeschlagen: {}", v["error_description"].as_str().unwrap_or("unbekannt")),
        }
    }
}

pub fn refresh(client_id: &str, refresh_token: &str) -> Result<(String, String)> {
    let (st, v) = net::post_form(
        &format!("{LOGIN}/token"),
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ],
    )?;
    if st != 200 {
        bail!("Token-Erneuerung fehlgeschlagen, bitte Account neu anmelden ({})", v["error"].as_str().unwrap_or("?"));
    }
    Ok((
        v["access_token"].as_str().unwrap_or_default().into(),
        v["refresh_token"].as_str().unwrap_or(refresh_token).into(),
    ))
}

/// Microsoft access token -> Xbox Live -> XSTS -> Minecraft services. Returns (mc_token, name, uuid).
pub fn minecraft_login(ms_access: &str) -> Result<(String, String, String)> {
    let (st, xbl) = net::post_json(
        "https://user.auth.xboxlive.com/user/authenticate",
        json!({
            "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_access}")},
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }),
    )?;
    if st != 200 {
        bail!("Xbox-Live-Anmeldung fehlgeschlagen (HTTP {st})");
    }
    let xbl_token = xbl["Token"].as_str().unwrap_or_default().to_string();

    let (st, xsts) = net::post_json(
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        json!({
            "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl_token]},
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }),
    )?;
    if st != 200 {
        match xsts["XErr"].as_u64() {
            Some(2148916233) => bail!("Dieses Microsoft-Konto hat kein Xbox-Profil"),
            Some(2148916238) => bail!("Kinderkonto: muss einer Microsoft-Familie hinzugef\u{fc}gt werden"),
            _ => bail!("XSTS-Anmeldung fehlgeschlagen (HTTP {st})"),
        }
    }
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().unwrap_or_default();
    let xsts_token = xsts["Token"].as_str().unwrap_or_default();

    let (st, mc) = net::post_json(
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({"identityToken": format!("XBL3.0 x={uhs};{xsts_token}")}),
    )?;
    if st != 200 {
        bail!("Minecraft-Anmeldung fehlgeschlagen (HTTP {st})");
    }
    let token = mc["access_token"].as_str().unwrap_or_default().to_string();

    let (st, prof): (u16, Value) = net::get_auth("https://api.minecraftservices.com/minecraft/profile", &token)?;
    if st == 404 {
        bail!("Auf diesem Konto ist Minecraft Java Edition nicht gekauft");
    }
    if st != 200 {
        bail!("Profilabfrage fehlgeschlagen (HTTP {st})");
    }
    Ok((
        token,
        prof["name"].as_str().unwrap_or_default().into(),
        prof["id"].as_str().unwrap_or_default().into(),
    ))
}
