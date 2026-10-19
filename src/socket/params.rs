//! Parameter parsing for the tab, pane and split socket methods. Pure functions, so the rules are
//! unit-tested without GTK. A parse error is answered with `invalid_params` before the request
//! reaches the GTK thread.

use crate::split_engine::FocusDirection;
use serde_json::Value;

/// Pixels pane.resize moves a divider by when the request gives no `px`.
pub const DEFAULT_RESIZE_PX: u64 = 40;
/// Largest `px` pane.resize accepts.
pub const MAX_RESIZE_PX: u64 = 10_000;

/// A tab named by its 0-based position (as tab.list reports it) or by its tab id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabRef {
    Index(usize),
    Id(u64),
}

/// `index` or `id` (not both) naming a tab. Neither gives None, which means the selected tab.
pub fn tab_ref(params: &Value) -> Result<Option<TabRef>, String> {
    let index = present(params, "index");
    let id = present(params, "id");
    match (index, id) {
        (Some(_), Some(_)) => Err("give either index or id, not both".into()),
        (Some(v), None) => Ok(Some(TabRef::Index(uint(v, "index")? as usize))),
        (None, Some(v)) => Ok(Some(TabRef::Id(uint(v, "id")?))),
        (None, None) => Ok(None),
    }
}

/// Optional `workspace` (UUID string). Missing or null means the active workspace.
pub fn workspace_uuid(params: &Value) -> Result<Option<String>, String> {
    optional_string(params, "workspace")
}

/// Optional `id` naming a pane by UUID. Missing or null means the focused pane.
pub fn pane_uuid(params: &Value) -> Result<Option<String>, String> {
    optional_string(params, "id")
}

/// `from` and `to` (0-based tab positions) for tab.move.
pub fn tab_move(params: &Value) -> Result<(usize, usize), String> {
    let from = uint(required(params, "from")?, "from")? as usize;
    let to = uint(required(params, "to")?, "to")? as usize;
    Ok((from, to))
}

/// `name` for tab.rename. Surrounding whitespace is dropped and a blank name is rejected.
pub fn tab_name(params: &Value) -> Result<String, String> {
    match params.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        _ => Err("name must be a non-empty string".into()),
    }
}

/// `left`, `right`, `up` or `down`, in any case.
pub fn direction(text: &str) -> Result<FocusDirection, String> {
    match text.to_ascii_lowercase().as_str() {
        "left" => Ok(FocusDirection::Left),
        "right" => Ok(FocusDirection::Right),
        "up" => Ok(FocusDirection::Up),
        "down" => Ok(FocusDirection::Down),
        _ => Err(format!("direction must be left, right, up or down, not '{text}'")),
    }
}

/// `direction` (required) and `px` (optional, 1 to MAX_RESIZE_PX) for pane.resize.
pub fn resize(params: &Value) -> Result<(FocusDirection, i32), String> {
    let text = required(params, "direction")?
        .as_str()
        .ok_or_else(|| "direction must be a string".to_string())?;
    let dir = direction(text)?;
    let px = match present(params, "px") {
        Some(v) => uint(v, "px")?,
        None => DEFAULT_RESIZE_PX,
    };
    if px == 0 || px > MAX_RESIZE_PX {
        return Err(format!("px must be between 1 and {MAX_RESIZE_PX}"));
    }
    Ok((dir, px as i32))
}

/// The value under `name`, treating an explicit null as absent.
fn present<'a>(params: &'a Value, name: &str) -> Option<&'a Value> {
    params.get(name).filter(|v| !v.is_null())
}

fn required<'a>(params: &'a Value, name: &str) -> Result<&'a Value, String> {
    present(params, name).ok_or_else(|| format!("{name} is required"))
}

fn uint(v: &Value, name: &str) -> Result<u64, String> {
    v.as_u64().ok_or_else(|| format!("{name} must be a non-negative integer"))
}

fn optional_string(params: &Value, name: &str) -> Result<Option<String>, String> {
    match present(params, name) {
        None => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{name} must be a non-empty string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tab_ref_takes_index_or_id() {
        assert_eq!(tab_ref(&json!({"index": 2})), Ok(Some(TabRef::Index(2))));
        assert_eq!(tab_ref(&json!({"id": 7})), Ok(Some(TabRef::Id(7))));
        assert_eq!(tab_ref(&json!({})), Ok(None));
        // An explicit null counts as absent.
        assert_eq!(tab_ref(&json!({"index": null, "id": 3})), Ok(Some(TabRef::Id(3))));
    }

    #[test]
    fn tab_ref_rejects_bad_values() {
        assert!(tab_ref(&json!({"index": 1, "id": 1})).is_err());
        assert!(tab_ref(&json!({"index": -1})).is_err());
        assert!(tab_ref(&json!({"index": "2"})).is_err());
        assert!(tab_ref(&json!({"id": 1.5})).is_err());
    }

    #[test]
    fn tab_move_needs_both_positions() {
        assert_eq!(tab_move(&json!({"from": 0, "to": 3})), Ok((0, 3)));
        assert!(tab_move(&json!({"from": 0})).is_err());
        assert!(tab_move(&json!({"to": 1})).is_err());
        assert!(tab_move(&json!({"from": -1, "to": 1})).is_err());
    }

    #[test]
    fn tab_name_is_trimmed_and_non_blank() {
        assert_eq!(tab_name(&json!({"name": "  build  "})), Ok("build".to_string()));
        assert!(tab_name(&json!({"name": "   "})).is_err());
        assert!(tab_name(&json!({})).is_err());
        assert!(tab_name(&json!({"name": 5})).is_err());
    }

    #[test]
    fn direction_is_case_insensitive() {
        assert_eq!(direction("Left"), Ok(FocusDirection::Left));
        assert_eq!(direction("DOWN"), Ok(FocusDirection::Down));
        assert!(direction("diagonal").is_err());
    }

    #[test]
    fn resize_defaults_to_40_px() {
        assert_eq!(resize(&json!({"direction": "right"})), Ok((FocusDirection::Right, 40)));
        assert_eq!(resize(&json!({"direction": "up", "px": 120})), Ok((FocusDirection::Up, 120)));
    }

    #[test]
    fn resize_rejects_missing_or_out_of_range_values() {
        assert!(resize(&json!({"px": 40})).is_err());
        assert!(resize(&json!({"direction": "left", "px": 0})).is_err());
        assert!(resize(&json!({"direction": "left", "px": MAX_RESIZE_PX + 1})).is_err());
        assert!(resize(&json!({"direction": "left", "px": "wide"})).is_err());
        assert!(resize(&json!({"direction": 3})).is_err());
    }

    #[test]
    fn workspace_and_pane_uuids_are_optional_strings() {
        assert_eq!(workspace_uuid(&json!({})), Ok(None));
        assert_eq!(workspace_uuid(&json!({"workspace": null})), Ok(None));
        assert_eq!(workspace_uuid(&json!({"workspace": " abc "})), Ok(Some("abc".to_string())));
        assert!(workspace_uuid(&json!({"workspace": ""})).is_err());
        assert_eq!(pane_uuid(&json!({"id": "p1"})), Ok(Some("p1".to_string())));
        assert!(pane_uuid(&json!({"id": 4})).is_err());
    }
}
