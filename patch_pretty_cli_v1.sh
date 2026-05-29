#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

# Replace final response call
s = s.replace(
    "print_response(response);",
    "print_response(&cmd, response);"
)

# Replace print_response function
start = s.index("fn print_response")
new_func = r'''fn print_response(cmd: &str, resp: ControlResponse) {
    if !resp.ok {
        println!("{}", serde_json::to_string_pretty(&resp.error).unwrap());
        return;
    }

    let value = resp.result.unwrap_or_else(|| serde_json::json!({}));

    match cmd {
        "status" => {
            println!("🐺 Werewolf Status");
            println!("  mode:         {}", value["mode"].as_str().unwrap_or("unknown"));
            println!("  pelt ready:   {}", value["pelt_ready"].as_bool().unwrap_or(false));
            println!("  packmates:    {}", value["packmates"].as_u64().unwrap_or(0));
            println!("  active fangs: {}", value["active_fangs"].as_u64().unwrap_or(0));
            println!("  silver:       {}", value["silver"].as_str().unwrap_or("unknown"));
            println!("  hide:         {}", value["hide"].as_str().unwrap_or("unknown"));
        }

        "den.info" => {
            println!("🏠 Den Info");
            println!("  socket:        {}", value["socket"].as_str().unwrap_or(""));
            println!("  home:          {}", value["home"].as_str().unwrap_or(""));
            println!("  listen:        {}", value["listen"].as_str().unwrap_or(""));
            println!("  mode:          {}", value["mode"].as_str().unwrap_or("unknown"));
            println!("  pelt ready:    {}", value["pelt_ready"].as_bool().unwrap_or(false));
            println!("  packmates:     {}", value["packmates"].as_u64().unwrap_or(0));
            println!("  fang profiles: {}", value["fang_profiles"].as_u64().unwrap_or(0));
            println!("  active fangs:  {}", value["active_fangs"].as_u64().unwrap_or(0));
            println!("  silver:        {}", value["silver"].as_str().unwrap_or("unknown"));
            println!("  hide:          {}", value["hide"].as_str().unwrap_or("unknown"));
        }

        "pelt.init" | "pelt.fingerprint" => {
            println!("🐾 Pelt");
            println!("  fingerprint: {}", value["fingerprint"].as_str().unwrap_or(""));
            if let Some(saved_to) = value["saved_to"].as_str() {
                println!("  saved to:     {}", saved_to);
            }
        }

        "pack.list" => {
            println!("🐾 Packmates");

            if let Some(peers) = value.as_array() {
                if peers.is_empty() {
                    println!("  none");
                }

                for peer in peers {
                    println!("  - {}", peer["name"].as_str().unwrap_or("unnamed"));
                    println!("      fingerprint: {}", peer["fingerprint"].as_str().unwrap_or(""));
                    println!("      address:     {}", peer["address"].as_str().unwrap_or(""));
                    println!("      trust:       {}", peer["trust"].as_str().unwrap_or(""));
                }
            }
        }

        "pack.add" | "pack.remove" => {
            println!("🐾 Pack");
            println!("  status:    {}", value["status"].as_str().unwrap_or(""));
            println!("  packmates: {}", value["packmates"].as_u64().unwrap_or(0));
        }

        "fang.list" => {
            println!("🦷 Active Fangs");

            if let Some(fangs) = value.as_array() {
                if fangs.is_empty() {
                    println!("  none");
                }

                for fang in fangs {
                    println!("  - {}", fang["id"].as_str().unwrap_or(""));
                    println!("      peer:   {}", fang["peer"].as_str().unwrap_or(""));
                    println!("      local:  {}", fang["local"].as_str().unwrap_or(""));
                    println!("      remote: {}", fang["remote"].as_str().unwrap_or(""));
                    println!("      state:  {}", fang["state"].as_str().unwrap_or(""));
                }
            }
        }

        "fang.open" | "fang.open_profile" => {
            println!("🦷 Fang Opened");
            println!("  id:     {}", value["fang_id"].as_str().unwrap_or(""));
            println!("  peer:   {}", value["peer"].as_str().unwrap_or(""));
            println!("  local:  {}", value["local"].as_str().unwrap_or(""));
            println!("  remote: {}", value["remote"].as_str().unwrap_or(""));
            println!("  state:  {}", value["state"].as_str().unwrap_or(""));
        }

        "fang.close" => {
            println!("🦷 Fang Closed");
            println!("  status:       {}", value["status"].as_str().unwrap_or(""));
            println!("  active fangs: {}", value["active_fangs"].as_u64().unwrap_or(0));
        }

        "fang.profile.list" => {
            println!("🦷 Fang Profiles");

            if let Some(profiles) = value.as_array() {
                if profiles.is_empty() {
                    println!("  none");
                }

                for profile in profiles {
                    println!("  - {}", profile["name"].as_str().unwrap_or(""));
                    println!("      peer:   {}", profile["peer"].as_str().unwrap_or(""));
                    println!("      local:  {}", profile["local"].as_str().unwrap_or(""));
                    println!("      remote: {}", profile["remote"].as_str().unwrap_or(""));
                }
            }
        }

        "fang.profile.add" | "fang.profile.remove" => {
            println!("🦷 Fang Profile");
            println!("  status:   {}", value["status"].as_str().unwrap_or(""));
            if let Some(name) = value["name"].as_str() {
                println!("  name:     {}", name);
            }
            println!("  profiles: {}", value["profiles"].as_u64().unwrap_or(0));
        }

        "silver.trigger" | "silver.reset" => {
            println!("🥈 Silver");
            println!("  status:  {}", value["status"].as_str().unwrap_or(""));
            println!("  message: {}", value["message"].as_str().unwrap_or(""));
        }

        _ => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
        }
    }
}
'''
s = s[:start] + new_func
p.write_text(s)
PY

echo "✨ Pretty CLI v1 patch applied."
