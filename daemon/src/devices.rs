//! Audio device discovery and resolution through PipeWire (`pw-dump`).
//!
//! Why do we resolve the device explicitly instead of letting `pw-record`/
//! `pw-play` use "auto"? Because with Bluetooth the graph changes while the app
//! is running: WirePlumber may have a headset saved as the default output
//! (`default.configured.audio.sink`) that is currently **not connected**, or
//! expose only the output (A2DP profile) and not the microphone (HSP/HFP).
//! Resolving the node manually lets us: (a) show which device is actually being
//! used, and (b) return a clear error instead of listening to silence.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Node {
    pub id: u64,
    pub name: String,
    pub desc: String,
    pub class: String,
}

impl Node {
    pub fn is_source(&self) -> bool {
        self.class.starts_with("Audio/Source")
    }

    pub fn is_sink(&self) -> bool {
        self.class.starts_with("Audio/Sink")
    }

    /// A Bluetooth node (headset/phone).
    pub fn is_bluetooth(&self) -> bool {
        self.name.starts_with("bluez_")
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub name: String,
    pub desc: String,
    pub api: String,
    pub profile: String,
}

/// Device entry consumed by the panel. `name` is passed to
/// `--mic-device`/`--speaker-device`; `description` is what is displayed.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceEntry {
    pub name: String,
    pub description: String,
    pub default: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceSnapshot {
    pub inputs: Vec<DeviceEntry>,
    pub outputs: Vec<DeviceEntry>,
}

/// The Duet virtual nodes are not valid choices: the microphone and
/// speaker that Meet already uses must not appear in the panel selectors.

fn is_virtual_duet_node(name: &str) -> bool {
    name.starts_with("duet_mic") || name.starts_with("duet_meet")
}

impl DeviceEntry {
    fn from_node(node: &Node, default: bool) -> Self {
        let description = if node.desc.is_empty() {
            node.name.clone()
        } else {
            node.desc.clone()
        };
        Self {
            name: node.name.clone(),
            description,
            default,
        }
    }
}

/// Panel list: real inputs and outputs, without duplicates or the Duet
/// virtual nodes themselves.
pub fn snapshot() -> Result<DeviceSnapshot> {
    let g = dump()?;
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for n in &g.nodes {
        if n.name.is_empty() || is_virtual_duet_node(&n.name) {
            continue;
        }
        if n.is_source() {
            let default = g.default_source.as_deref() == Some(&n.name);
            inputs.push(DeviceEntry::from_node(n, default));
        } else if n.is_sink() {
            let default = g.default_sink.as_deref() == Some(&n.name);
            outputs.push(DeviceEntry::from_node(n, default));
        }
    }
    Ok(DeviceSnapshot { inputs, outputs })
}

/// JSON output for `duet --devices-json` (used by the panel when the
/// daemon is not running yet).
pub fn snapshot_json() -> Result<String> {
    Ok(serde_json::to_string(&snapshot()?)?)
}

pub struct Graph {
    pub nodes: Vec<Node>,
    pub devices: Vec<Device>,
    pub default_source: Option<String>,
    pub default_sink: Option<String>,
    /// Output "remembered" by WirePlumber even if it is not connected right now.
    pub configured_sink: Option<String>,
    pub configured_source: Option<String>,
}

fn str_prop(props: &Value, key: &str) -> Option<String> {
    props.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// Run `pw-dump` and build the current audio graph.
pub fn dump() -> Result<Graph> {
    let out = Command::new("pw-dump")
        .output()
        .context("could not run pw-dump (is PipeWire running?)")?;
    if !out.status.success() {
        bail!("pw-dump failed with {}", out.status);
    }
    let json: Value = serde_json::from_slice(&out.stdout).context("pw-dump did not return JSON")?;

    let mut nodes = Vec::new();
    let mut devices = Vec::new();
    let (mut default_source, mut default_sink) = (None, None);
    let (mut configured_sink, mut configured_source) = (None, None);

    for obj in json.as_array().cloned().unwrap_or_default() {
        match obj.get("type").and_then(|t| t.as_str()) {
            Some("PipeWire:Interface:Node") => {
                let props = &obj["info"]["props"];
                let class = str_prop(props, "media.class").unwrap_or_default();
                if class.starts_with("Audio/Source") || class.starts_with("Audio/Sink") {
                    nodes.push(Node {
                        id: obj["id"].as_u64().unwrap_or(0),
                        name: str_prop(props, "node.name").unwrap_or_default(),
                        desc: str_prop(props, "node.description")
                            .or_else(|| str_prop(props, "node.nick"))
                            .unwrap_or_default(),
                        class,
                    });
                }
            }
            Some("PipeWire:Interface:Device") => {
                let props = &obj["info"]["props"];
                if str_prop(props, "media.class").as_deref() == Some("Audio/Device") {
                    devices.push(Device {
                        name: str_prop(props, "device.name").unwrap_or_default(),
                        desc: str_prop(props, "device.description").unwrap_or_default(),
                        api: str_prop(props, "device.api").unwrap_or_default(),
                        profile: str_prop(props, "device.profile.description").unwrap_or_default(),
                    });
                }
            }
            Some("PipeWire:Interface:Metadata") => {
                if str_prop(&obj["props"], "metadata.name").as_deref() != Some("default") {
                    continue;
                }
                for m in obj["metadata"].as_array().cloned().unwrap_or_default() {
                    let key = m["key"].as_str().unwrap_or("");
                    let name = m["value"]["name"].as_str().map(str::to_string);
                    match key {
                        "default.audio.source" => default_source = name,
                        "default.audio.sink" => default_sink = name,
                        "default.configured.audio.source" => configured_source = name,
                        "default.configured.audio.sink" => configured_sink = name,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Graph {
        nodes,
        devices,
        default_source,
        default_sink,
        configured_sink,
        configured_source,
    })
}

/// Print the graph and exit. This is the Bluetooth diagnostic tool.
pub fn list() -> Result<()> {
    let g = dump()?;

    println!("-- inputs (microphones) ----------------------------------");
    let sources: Vec<_> = g.nodes.iter().filter(|n| n.is_source()).collect();
    if sources.is_empty() {
        println!("   (none)");
    }
    for n in &sources {
        let mark = if g.default_source.as_deref() == Some(&n.name) {
            " [default]"
        } else {
            ""
        };
        println!("   {:>3}  {}{}", n.id, n.name, mark);
        println!(
            "        {}  ({})",
            n.desc,
            n.class.trim_start_matches("Audio/")
        );
    }

    println!("\n-- outputs (speakers/headphones) -------------------------");
    let sinks: Vec<_> = g.nodes.iter().filter(|n| n.is_sink()).collect();
    if sinks.is_empty() {
        println!("   (none)");
    }
    for n in &sinks {
        let mark = if g.default_sink.as_deref() == Some(&n.name) {
            " [default]"
        } else {
            ""
        };
        println!("   {:>3}  {}{}", n.id, n.name, mark);
        println!(
            "        {}  ({})",
            n.desc,
            n.class.trim_start_matches("Audio/")
        );
    }

    if !g.devices.is_empty() {
        println!("\n-- devices -----------------------------------------------");
        for d in &g.devices {
            println!("   {:<38} {}  [{}]", d.name, d.desc, d.api);
            if !d.profile.is_empty() && d.profile != "null" {
                println!("        profile: {}", d.profile);
            }
        }
    }

    // Bluetooth diagnostics: this is the case that breaks most often.
    let has_bt_sink = sinks.iter().any(|n| n.is_bluetooth());
    let has_bt_src = sources.iter().any(|n| n.is_bluetooth());
    let mentions_bt = g
        .configured_sink
        .as_deref()
        .is_some_and(|s| s.starts_with("bluez_"))
        || g.configured_source
            .as_deref()
            .is_some_and(|s| s.starts_with("bluez_"))
        || g.devices.iter().any(|d| d.api == "bluez");
    if mentions_bt && !(has_bt_sink && has_bt_src) {
        println!("\n-- Bluetooth notice --------------------------------------");
        if !has_bt_sink {
            println!(
                "   A Bluetooth headset is configured ({:?})",
                g.configured_sink
            );
            println!("   but it is not connected right now.");
        }
        if has_bt_sink && !has_bt_src {
            println!("   The headset exposes output (A2DP) but NO microphone.");
            println!("   To use its microphone, switch the profile to Headset:");
            println!("     wpctl set-profile <device-id> headset-head-unit");
            println!("   (or select the HSP/HFP profile in pavucontrol).");
        }
    }

    println!("\nPass these names with --mic-device / --speaker-device.");
    Ok(())
}

/// Find a node by name, then by description.
///
/// A Bluetooth node name includes the MAC and an index that changes with the
/// profile (`bluez_output.00_BB_43_C3_2C_8F.1`), so asking for it from memory is
/// awkward. That is why we also accept the description ("Redmi Buds 6 Play",
/// "Built-in Audio"), case-insensitively.
fn find<'a>(nodes: &'a [Node], want: &str, source: bool) -> Option<&'a Node> {
    let candidates: Vec<&Node> = nodes
        .iter()
        .filter(|n| if source { n.is_source() } else { n.is_sink() })
        .collect();
    let lowered = want.to_lowercase();
    candidates
        .iter()
        .find(|n| n.name == want)
        .or_else(|| candidates.iter().find(|n| n.name.contains(want)))
        .or_else(|| {
            candidates
                .iter()
                .find(|n| n.desc.to_lowercase().contains(&lowered))
        })
        .copied()
}

fn hint_if_bt_missing(g: &Graph, want: Option<&str>, source: bool) -> String {
    let asked_bt = want.is_some_and(|w| w.starts_with("bluez_"));
    let bt_present = asked_bt
        || g.devices.iter().any(|d| d.api == "bluez")
        || g.nodes.iter().any(|n| n.is_bluetooth())
        || g.configured_sink
            .as_deref()
            .is_some_and(|n| n.starts_with("bluez_"))
        || g.configured_source
            .as_deref()
            .is_some_and(|n| n.starts_with("bluez_"));
    if !bt_present {
        return String::new();
    }
    if source {
        "\n   Hint: your Bluetooth headset may be in A2DP profile (no microphone).\n   \
         Switch it to Headset/HFP with:  wpctl set-profile <id> headset-head-unit\n   \
         Check with:  duet --list-devices"
            .to_string()
    } else {
        "\n   Hint: the Bluetooth headset is not connected. Check with:\n   \
         duet --list-devices"
            .to_string()
    }
}

/// Resolve the input to use. `want` = name requested by the user (or None).
///
/// If the node does not exist yet (typical with Bluetooth, which takes time to
/// switch profiles), wait up to `wait` before giving up.
pub fn resolve_source(want: Option<&str>, wait: Duration) -> Result<Node> {
    resolve(want, wait, true)
}

pub fn resolve_sink(want: Option<&str>, wait: Duration) -> Result<Node> {
    resolve(want, wait, false)
}

fn resolve(want: Option<&str>, wait: Duration, source: bool) -> Result<Node> {
    let deadline = Instant::now() + wait;
    loop {
        let g = dump()?;
        let found = match want {
            Some(w) => find(&g.nodes, w, source),
            None => {
                let default = if source {
                    g.default_source.as_deref()
                } else {
                    g.default_sink.as_deref()
                };
                default.and_then(|d| find(&g.nodes, d, source)).or_else(|| {
                    g.nodes
                        .iter()
                        .find(|n| if source { n.is_source() } else { n.is_sink() })
                })
            }
        };
        if let Some(n) = found {
            return Ok(n.clone());
        }

        if Instant::now() >= deadline {
            let kind = if source {
                "no microphone"
            } else {
                "no audio output"
            };
            let named = match want {
                Some(w) => format!(" '{w}'"),
                None => String::new(),
            };
            bail!(
                "could not find {kind}{named}. Is it connected?\n   \
                 Check available devices with: duet --list-devices{}",
                hint_if_bt_missing(&g, want, source)
            );
        }
        thread::sleep(Duration::from_millis(300));
    }
}
