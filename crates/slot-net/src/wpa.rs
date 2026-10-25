//! Talking to wpa_supplicant: the config slot writes for it, and what `wpa_cli` prints back.

use crate::config::Network;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    pub ssid: String,
    pub bssid: String,
    pub freq: u32,
    pub signal_dbm: i32,
    pub secured: bool,
    /// WPA3 only, which wpa_supplicant on these boards may not speak.
    pub sae_only: bool,
}

impl ScanResult {
    /// 0 to 4 bars, the way a phone draws them.
    pub fn bars(&self) -> u8 {
        match self.signal_dbm {
            s if s >= -55 => 4,
            s if s >= -66 => 3,
            s if s >= -77 => 2,
            s if s >= -88 => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WpaStatus {
    pub state: String,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub ip: Option<String>,
}

impl WpaStatus {
    pub fn completed(&self) -> bool {
        self.state == "COMPLETED"
    }
}

/// The supplicant config for `networks`, the first one most preferred.
///
/// SSIDs go in as hex so any byte is safe. Passphrases go in quoted; wpa_supplicant takes
/// everything up to the last quote, so a quote inside one is fine. A 64 digit hex string is
/// a raw PSK and goes in bare.
pub fn supplicant_conf(ctrl_dir: &str, networks: &[Network], country: Option<&str>) -> String {
    let mut s = format!("ctrl_interface={ctrl_dir}\nupdate_config=0\nap_scan=1\n");
    if let Some(cc) =
        country.filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()))
    {
        s += &format!("country={}\n", cc.to_ascii_uppercase());
    }
    let count = networks.len();
    for (i, n) in networks.iter().enumerate() {
        s += "\nnetwork={\n";
        s += &format!("\tssid={}\n", hex(n.ssid.as_bytes()));
        if n.hidden {
            s += "\tscan_ssid=1\n";
        }
        if n.psk.is_empty() {
            s += "\tkey_mgmt=NONE\n";
        } else if n.psk.len() == 64 && n.psk.chars().all(|c| c.is_ascii_hexdigit()) {
            s += "\tkey_mgmt=WPA-PSK\n";
            s += &format!("\tpsk={}\n", n.psk);
        } else {
            s += "\tkey_mgmt=WPA-PSK\n";
            s += &format!("\tpsk=\"{}\"\n", n.psk.replace('\n', ""));
        }
        s += &format!("\tpriority={}\n", count - i);
        s += "}\n";
    }
    s
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// wpa_supplicant needs a passphrase of 8 to 63 printable ASCII characters, or 64 hex digits.
pub fn valid_passphrase(psk: &str) -> bool {
    let n = psk.chars().count();
    let printable = psk.chars().all(|c| (' '..='~').contains(&c));
    (printable && (8..=63).contains(&n)) || (n == 64 && psk.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `wpa_cli scan_results`: a header line, then tab separated
/// `bssid  frequency  signal  flags  ssid`. Hidden networks (empty SSIDs) are dropped and each
/// name keeps only its strongest access point.
pub fn parse_scan(text: &str) -> Vec<ScanResult> {
    let mut out: Vec<ScanResult> = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.splitn(5, '\t').collect();
        if cols.len() < 4 || !cols[0].contains(':') {
            continue;
        }
        let ssid = cols.get(4).map(|s| unescape(s)).unwrap_or_default();
        if ssid.is_empty() || ssid.chars().all(|c| c == '\0') {
            continue;
        }
        let flags = cols[3];
        let wpa = flags.contains("WPA") || flags.contains("RSN");
        let psk_ok = flags.contains("PSK") || flags.contains("FT/PSK");
        let r = ScanResult {
            ssid,
            bssid: cols[0].to_string(),
            freq: cols[1].trim().parse().unwrap_or(0),
            signal_dbm: cols[2].trim().parse().unwrap_or(-100),
            secured: wpa || flags.contains("WEP"),
            sae_only: flags.contains("SAE") && !psk_ok,
        };
        match out.iter_mut().find(|o| o.ssid == r.ssid) {
            Some(o) if o.signal_dbm < r.signal_dbm => *o = r,
            Some(_) => {}
            None => out.push(r),
        }
    }
    out.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm).then(a.ssid.cmp(&b.ssid)));
    out
}

/// `wpa_cli status`, `key=value` per line.
pub fn parse_status(text: &str) -> WpaStatus {
    let mut s = WpaStatus::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "wpa_state" => s.state = v.trim().to_string(),
            "ssid" => s.ssid = Some(unescape(v)),
            "bssid" => s.bssid = Some(v.trim().to_string()),
            "ip_address" => s.ip = Some(v.trim().to_string()).filter(|v| !v.is_empty()),
            _ => {}
        }
    }
    s
}

/// `wpa_cli list_networks`: `id  ssid  bssid  flags`, returns the id of `ssid`.
pub fn network_id(list: &str, ssid: &str) -> Option<u32> {
    list.lines().find_map(|line| {
        let mut cols = line.split('\t');
        let id = cols.next()?.trim().parse().ok()?;
        (unescape(cols.next()?) == ssid).then_some(id)
    })
}

/// wpa_cli prints SSIDs through printf_encode: `\\`, `\"`, `\e`, `\n`, `\r`, `\t` and `\xNN`.
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let (byte, used) = match b[i + 1] {
                b'\\' => (b'\\', 2),
                b'"' => (b'"', 2),
                b'n' => (b'\n', 2),
                b'r' => (b'\r', 2),
                b't' => (b'\t', 2),
                b'e' => (0x1b, 2),
                b'x' if i + 3 < b.len() => match std::str::from_utf8(&b[i + 2..i + 4])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(v) => (v, 4),
                    None => (b'\\', 1),
                },
                _ => (b'\\', 1),
            };
            out.push(byte);
            i += used;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAN: &str = "bssid / frequency / signal level / flags / ssid\n\
        aa:bb:cc:dd:ee:01\t2437\t-71\t[WPA2-PSK-CCMP][ESS]\tHome\n\
        aa:bb:cc:dd:ee:02\t5180\t-50\t[WPA2-PSK-CCMP][ESS]\tHome\n\
        aa:bb:cc:dd:ee:03\t2412\t-80\t[ESS]\tCoffee Shop\n\
        aa:bb:cc:dd:ee:04\t2412\t-60\t[WPA2-PSK-CCMP][ESS]\t\n\
        aa:bb:cc:dd:ee:05\t2462\t-65\t[WPA2-SAE-CCMP][ESS]\tNew\\xc3\\xa9\n\
        aa:bb:cc:dd:ee:06\t2462\t-90\t[WPA2-PSK+SAE-CCMP][ESS]\tMixed\n";

    #[test]
    fn scan_keeps_the_strongest_of_each_name_and_skips_hidden() {
        let r = parse_scan(SCAN);
        let names: Vec<&str> = r.iter().map(|r| r.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Newé", "Coffee Shop", "Mixed"]);
        assert_eq!(r[0].signal_dbm, -50);
        assert_eq!(r[0].freq, 5180);
        assert!(r[0].secured);
        assert!(!r[2].secured);
        assert!(r[1].sae_only);
        assert!(!r[3].sae_only);
        assert_eq!(r[0].bars(), 4);
        assert_eq!(r[3].bars(), 0);
    }

    #[test]
    fn status_reads_state_name_and_address() {
        let s = parse_status(
            "bssid=aa:bb:cc:dd:ee:02\nfreq=5180\nssid=Home\nid=0\nmode=station\n\
             wpa_state=COMPLETED\nip_address=192.168.1.20\naddress=00:11:22:33:44:55\n",
        );
        assert!(s.completed());
        assert_eq!(s.ssid.as_deref(), Some("Home"));
        assert_eq!(s.ip.as_deref(), Some("192.168.1.20"));
        assert!(!parse_status("wpa_state=SCANNING\n").completed());
    }

    #[test]
    fn conf_writes_hex_names_quoted_passphrases_and_falling_priority() {
        let nets = [
            Network {
                ssid: "Home".into(),
                psk: "pa\"ss word".into(),
                hidden: false,
            },
            Network {
                ssid: "Open".into(),
                psk: String::new(),
                hidden: true,
            },
        ];
        let c = supplicant_conf("/run/slotnet", &nets, Some("us"));
        assert!(c.starts_with("ctrl_interface=/run/slotnet\n"));
        assert!(c.contains("country=US\n"));
        assert!(c.contains("ssid=486f6d65\n"));
        assert!(c.contains("psk=\"pa\"ss word\"\n"));
        assert!(c.contains("priority=2\n"));
        assert!(c.contains("ssid=4f70656e\n\tscan_ssid=1\n\tkey_mgmt=NONE\n\tpriority=1\n"));
    }

    #[test]
    fn a_raw_psk_goes_in_bare() {
        let raw = "a".repeat(64);
        let c = supplicant_conf(
            "/r",
            &[Network {
                ssid: "x".into(),
                psk: raw.clone(),
                hidden: false,
            }],
            None,
        );
        assert!(c.contains(&format!("psk={raw}\n")));
        assert!(!c.contains("country"));
    }

    #[test]
    fn passphrase_rules_match_wpa_supplicant() {
        assert!(valid_passphrase("12345678"));
        assert!(!valid_passphrase("1234567"));
        assert!(valid_passphrase(&"x".repeat(63)));
        assert!(!valid_passphrase(&"x".repeat(64)));
        assert!(valid_passphrase(&"ab".repeat(32)));
        assert!(!valid_passphrase("pässword"));
    }

    #[test]
    fn finds_a_network_id_by_name() {
        let list =
            "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tCaf\\xc3\\xa9\tany\t\n";
        assert_eq!(network_id(list, "Home"), Some(0));
        assert_eq!(network_id(list, "Café"), Some(1));
        assert_eq!(network_id(list, "Nope"), None);
    }

    #[test]
    fn unescape_handles_every_printf_encoding() {
        assert_eq!(unescape(r#"a\\b\"c\tq\x41"#), "a\\b\"c\tqA");
        assert_eq!(unescape(r"trailing\"), "trailing\\");
    }
}
