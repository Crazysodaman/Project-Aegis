//! Test-only module binary used to exercise module-provided tools through the
//! central Aegis tool executor. It is not a functional module.

use std::fs;
use std::thread;
use std::time::Duration;

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut tool = None;
    let mut input = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--aegis-tool" => {
                index += 1;
                tool = args.get(index).cloned();
            }
            "--input" => {
                index += 1;
                input = args.get(index).cloned();
            }
            "--output" => {
                index += 1;
                output = args.get(index).cloned();
            }
            _ => {}
        }
        index += 1;
    }
    let (Some(tool), Some(input), Some(output)) = (tool, input, output) else {
        std::process::exit(2);
    };
    let input_value: Value =
        serde_json::from_slice(&fs::read(input).expect("read input")).expect("valid input json");
    let response = match tool.as_str() {
        "echo" => serde_json::json!({"ok": true, "output": input_value}),
        "fail" => serde_json::json!({"ok": false, "error": "requested failure"}),
        "sleep" => {
            thread::sleep(Duration::from_secs(5));
            serde_json::json!({"ok": true, "output": {"slept": true}})
        }
        _ => serde_json::json!({"ok": false, "error": "unknown tool"}),
    };
    fs::write(output, response.to_string()).expect("write output");
}
