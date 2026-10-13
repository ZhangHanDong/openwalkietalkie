//! `docs/deckd-api.md`, **generated from `GET /v1/schema`** (design §4,
//! HOST-CONTRACT-W7 §5).
//!
//! The point is that it cannot drift. A hand-written API reference is a
//! second source of truth that nothing checks, and this one has the same
//! input as the endpoint a third party reads: `deck_model::schema::schema()`
//! for the types plus `api::dispatch::methods_json()` for the table, which is
//! exactly what `schema_handler` serves. [`render`] takes that object and
//! nothing else — no clock, no version, no environment — so the same schema
//! always produces the same bytes, and `tests/api_doc.rs` asserts that the
//! checked-in file is those bytes, the way `gate_ratchet` pins the counts.
//!
//! What it deliberately does **not** do is add prose. Anything a reader needs
//! that is not in the schema belongs in `docs/host-service.md` §4, which this
//! file links to; anything written here would be the drift it exists to
//! prevent.
use serde_json::Value;
use std::fmt::Write as _;

/// The exact command that rewrites the file, printed by the generator and by
/// the test that fails when the file is stale. The flags are the workspace's
/// mandatory ones (`host/.cargo/config.toml` deliberately sets no target).
pub const REGENERATE: &str = "cargo run --locked -p deckd --target aarch64-apple-darwin \
                              --target-dir target/host --manifest-path host/Cargo.toml -- \
                              api-doc > docs/deckd-api.md";

/// The schema this daemon serves on `GET /v1/schema`: the model's own
/// schema plus the dispatch table.
pub fn schema() -> Value {
    let mut schema = deck_model::schema::schema();
    schema["methods"] = Value::Array(crate::api::dispatch::methods_json());
    schema
}

/// The whole document, from the schema object and nothing else.
pub fn render(schema: &Value) -> String {
    // The GUI enables serde_json/preserve_order for the full workspace.
    // Documentation must be identical with or without that feature.
    let mut canonical = schema.clone();
    canonical.sort_all_objects();
    let schema = &canonical;
    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(&methods(schema));
    out.push_str(&events(schema));
    out.push_str(&top_level(schema));
    out.push_str(&types(schema));
    out
}

fn preamble() -> String {
    format!(
        "# `deckd` — the v1 API\n\
         \n\
         <!-- GENERATED FILE — DO NOT EDIT. -->\n\
         \n\
         This file is generated from what the running daemon serves on `GET /v1/schema`\n\
         (the JSON Schema of every method, event and type, from the one Rust source in\n\
         `deck-model`, plus the dispatch table). Editing it by hand is pointless: the\n\
         `deckd` test `the_checked_in_api_document_is_what_the_schema_produces` compares\n\
         the file with the generator's output on every run. To change it, change the\n\
         Rust types or the method table and regenerate:\n\
         \n\
         ```sh\n\
         {REGENERATE}\n\
         ```\n\
         \n\
         The prose that explains *why* the API is shaped this way — transports, principals\n\
         and scopes, content classes, the delivery contract, the error codes — is\n\
         [`docs/host-service.md` §4](host-service.md#4-the-open-api). Nothing is written\n\
         here that the schema does not carry, because anything written here could go\n\
         stale and nothing would notice.\n"
    )
}

/// The method table: what a client may call, what scope it needs, and where
/// the answer comes from.
fn methods(schema: &Value) -> String {
    let variants = request_variants(schema);
    let mut out = String::from(
        "\n## Methods\n\
         \n\
         `scope` is the least scope a principal must hold (§4.2); a method with no scope\n\
         is answerable before the handshake. `answered by` is `store` for a method the\n\
         store's reducer answers from the snapshot and `core` for one the core thread\n\
         answers out of band. `params` links the type the request carries.\n\
         \n\
         | method | scope | answered by | params |\n\
         |---|---|---|---|\n",
    );
    for method in schema["methods"].as_array().into_iter().flatten() {
        let name = method["name"].as_str().unwrap_or_default();
        let scope = match method["scope"].as_str() {
            Some(scope) => format!("`{scope}`"),
            None => "— (no scope)".into(),
        };
        let answered = if method["reducer"].as_bool().unwrap_or(false) {
            "store"
        } else {
            "core"
        };
        let params = variants
            .iter()
            .find(|(variant, _)| variant == name)
            .map(|(_, params)| params.clone())
            .unwrap_or_else(|| "— (not in the `Request` enum)".into());
        let _ = writeln!(out, "| `{name}` | {scope} | {answered} | {params} |");
    }
    out
}

/// `[(method name, rendered params type)]` from the `Request` schema's
/// `oneOf`, whose every variant is `{method: {const}, params: …}`.
fn request_variants(schema: &Value) -> Vec<(String, String)> {
    let request = &schema["request"];
    let variants = request["oneOf"]
        .as_array()
        .or_else(|| request["anyOf"].as_array());
    variants
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            let name = variant["properties"]["method"]["const"].as_str()?;
            let params = &variant["properties"]["params"];
            let rendered = if params.is_null() {
                "— (none)".into()
            } else if params.get("$ref").is_some() {
                type_name(params)
            } else {
                // An inline object: name its fields rather than say
                // "object", because that is the whole call.
                inline_object(params)
            };
            Some((name.to_string(), rendered))
        })
        .collect()
}

/// `{agent, card}` for a params object written inline in the enum.
fn inline_object(params: &Value) -> String {
    let Some(props) = params["properties"].as_object() else {
        // `{"type": "object"}` with no properties is a call that takes
        // nothing — `voice.cancel`, not "some object".
        return if params["type"] == "object" {
            "{}".into()
        } else {
            type_name(params)
        };
    };
    let required: Vec<&str> = params["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut names: Vec<&String> = props.keys().collect();
    names.sort();
    let fields: Vec<String> = names
        .iter()
        .map(|name| {
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!("{name}{optional}: {}", type_name(&props[*name]))
        })
        .collect();
    format!("{{{}}}", fields.join(", "))
}

fn events(schema: &Value) -> String {
    let mut out = String::from(
        "\n## Events\n\
         \n\
         Server push is a JSON-RPC notification `{\"method\":\"event\", \"params\": Event}`\n\
         (§4.12). `data` is redacted per subscriber at fan-out, so what a given session\n\
         sees depends on its scopes.\n\n",
    );
    out.push_str(&object_table(&schema["event"], &schema["event"]));
    let kinds = schema["event"]["$defs"]["EventKind"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    out.push_str("\n`type` is one of:\n\n");
    for kind in kinds {
        if let Some(kind) = kind.as_str() {
            let _ = writeln!(out, "- `{kind}`");
        }
    }
    out
}

/// The three remaining top-level schemas: the snapshot a client starts from,
/// the link events the hid thread reports, and the error envelope.
fn top_level(schema: &Value) -> String {
    let mut out = String::new();
    for (title, key) in [
        ("Snapshot", "snapshot"),
        ("Errors", "error"),
        ("Link events", "link_event"),
    ] {
        let value = &schema[key];
        let _ = writeln!(out, "\n## {title}\n");
        if let Some(description) = value["description"].as_str() {
            let _ = writeln!(out, "{description}\n");
        }
        if value.get("oneOf").is_some() || value.get("anyOf").is_some() {
            out.push_str(&variant_list(value, value));
        } else {
            out.push_str(&object_table(value, value));
        }
    }
    out
}

/// Every named type, once, in one alphabetical list. The same `$def` appears
/// under several top-level schemas (`Ts` is in all of them) and a reader
/// wants one place to look it up.
fn types(schema: &Value) -> String {
    let mut defs: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for key in ["snapshot", "event", "request", "link_event", "error"] {
        if let Some(map) = schema[key]["$defs"].as_object() {
            for (name, value) in map {
                defs.entry(name.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    let mut out = format!(
        "\n## Types\n\nThe {} named types the schemas above refer to.\n",
        defs.len()
    );
    for (name, value) in &defs {
        let _ = writeln!(out, "\n### `{name}`\n");
        if let Some(description) = value["description"].as_str() {
            let _ = writeln!(out, "{}\n", description.replace('\n', " "));
        }
        if let Some(values) = value["enum"].as_array() {
            let list: Vec<String> = values
                .iter()
                .map(|v| match v.as_str() {
                    Some(s) => format!("`{s}`"),
                    None => format!("`{v}`"),
                })
                .collect();
            let _ = writeln!(out, "One of: {}.\n", list.join(", "));
        } else if value.get("oneOf").is_some() || value.get("anyOf").is_some() {
            out.push_str(&variant_list(value, value));
        } else if value["properties"].is_object() {
            out.push_str(&object_table(value, value));
        } else {
            let _ = writeln!(out, "Type: {}.\n", type_name(value));
        }
    }
    out
}

/// A `| field | type | required | default | description |` table.
fn object_table(value: &Value, _root: &Value) -> String {
    let Some(props) = value["properties"].as_object() else {
        return format!("Type: {}.\n", type_name(value));
    };
    let required: Vec<&str> = value["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut names: Vec<&String> = props.keys().collect();
    names.sort();
    let mut out = String::from(
        "| field | type | required | default | description |\n|---|---|---|---|---|\n",
    );
    for name in names {
        let field = &props[name];
        let default = match field.get("default") {
            None => String::new(),
            Some(Value::Null) => "`null`".into(),
            Some(v) => format!("`{v}`"),
        };
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} | {default} | {} |",
            type_name(field),
            if required.contains(&name.as_str()) {
                "yes"
            } else {
                "no"
            },
            field["description"]
                .as_str()
                .unwrap_or_default()
                .replace('\n', " ")
                .replace('|', "\\|")
        );
    }
    out
}

/// A `oneOf`/`anyOf` as a bullet per variant.
fn variant_list(value: &Value, _root: &Value) -> String {
    let variants = value["oneOf"]
        .as_array()
        .or_else(|| value["anyOf"].as_array());
    let mut out = String::new();
    for variant in variants.into_iter().flatten() {
        let shape = if variant["properties"].is_object() {
            inline_object(variant)
        } else {
            type_name(variant)
        };
        match variant["description"].as_str() {
            Some(description) => {
                let _ = writeln!(out, "- {shape} — {}", description.replace('\n', " "));
            }
            None => {
                let _ = writeln!(out, "- {shape}");
            }
        }
    }
    out.push('\n');
    out
}

/// One field's type, as a reader would write it: a `$ref` becomes a link to
/// the `## Types` entry, a union of `T` and `null` becomes `T?`, an array
/// becomes `T[]`.
pub fn type_name(field: &Value) -> String {
    if let Some(reference) = field["$ref"].as_str() {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        return format!("[`{name}`](#{})", anchor(name));
    }
    // `anyOf: [{$ref: T}, {type: null}]` is how schemars writes `Option<T>`.
    if let Some(list) = field["anyOf"]
        .as_array()
        .or_else(|| field["oneOf"].as_array())
    {
        let nullable = list.iter().any(|v| v["type"] == "null");
        let rest: Vec<String> = list
            .iter()
            .filter(|v| v["type"] != "null")
            .map(type_name)
            .collect();
        if rest.len() == 1 && nullable {
            return format!("{}?", rest[0]);
        }
        if rest.is_empty() {
            return "null".into();
        }
        return format!("{}{}", rest.join(" \\| "), if nullable { "?" } else { "" });
    }
    match &field["type"] {
        Value::String(one) if one == "array" => format!("{}[]", type_name(&field["items"])),
        Value::String(one) if one == "integer" || one == "number" => {
            field["format"].as_str().unwrap_or(one.as_str()).to_string()
        }
        Value::String(one) => one.clone(),
        Value::Array(list) => {
            let nullable = list.iter().any(|v| v == "null");
            let rest: Vec<String> = list
                .iter()
                .filter(|v| *v != "null")
                .filter_map(Value::as_str)
                .map(|t| {
                    if t == "integer" || t == "number" {
                        field["format"].as_str().unwrap_or(t).to_string()
                    } else {
                        t.to_string()
                    }
                })
                .collect();
            format!(
                "{}{}",
                rest.join(" \\| "),
                if nullable && !rest.is_empty() {
                    "?"
                } else {
                    ""
                }
            )
        }
        // No type discriminator at all: `true` is schemars for "any JSON"
        // (`serde_json::Value`), and so is a schema carrying nothing but a
        // `default` — an untyped `meta` field is any JSON, not an object.
        _ => "any".into(),
    }
}

/// A GitHub-flavoured heading anchor for `### \`Name\``.
fn anchor(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ref_becomes_a_link_and_an_option_becomes_a_question_mark() {
        assert_eq!(
            type_name(&serde_json::json!({"$ref": "#/$defs/AgentId"})),
            "[`AgentId`](#agentid)"
        );
        assert_eq!(
            type_name(&serde_json::json!({
                "anyOf": [{"$ref": "#/$defs/PidKind"}, {"type": "null"}]
            })),
            "[`PidKind`](#pidkind)?"
        );
        assert_eq!(
            type_name(&serde_json::json!({"type": ["integer", "null"], "format": "uint32"})),
            "uint32?"
        );
        assert_eq!(
            type_name(&serde_json::json!({"type": "array", "items": {"type": "string"}})),
            "string[]"
        );
        // schemars writes `serde_json::Value` as the schema `true`.
        assert_eq!(type_name(&serde_json::json!(true)), "any");
    }

    /// The document is a pure function of the schema: nothing in it comes
    /// from the clock, the environment or the package version, which is what
    /// lets a test compare it with a checked-in file.
    #[test]
    fn rendering_the_same_schema_twice_gives_the_same_bytes() {
        let schema = schema();
        assert_eq!(render(&schema), render(&schema));
    }

    /// Every method the daemon answers is in the table, with its scope — the
    /// table *is* `/v1/schema.methods`, so a method added without a row is a
    /// method the document cannot omit.
    #[test]
    fn every_dispatch_row_reaches_the_method_table() {
        let schema = schema();
        let doc = render(&schema);
        for method in schema["methods"].as_array().unwrap() {
            let name = method["name"].as_str().unwrap();
            assert!(doc.contains(&format!("| `{name}` |")), "{name} is missing");
        }
        // And a params type that is named in the table has an entry to link to.
        assert!(doc.contains("### `AgentRegister`"), "the types section");
        assert!(doc.contains(REGENERATE), "the regeneration command");
    }

    /// A method whose params are written inline in the `Request` enum is
    /// shown as its fields, not as the word "object": the fields are the
    /// whole call.
    #[test]
    fn an_inline_params_object_is_shown_as_its_fields() {
        let doc = render(&schema());
        let row = doc
            .lines()
            .find(|l| l.starts_with("| `agent.focus` |"))
            .expect("agent.focus is in the table");
        assert!(row.contains("{agent:"), "{row}");
    }
}
