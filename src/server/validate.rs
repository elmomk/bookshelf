use dioxus::prelude::ServerFnError;

const MAX_TEXT: usize = 500;

/// Short fields (titles, etc.). Counts characters, not bytes.
pub fn text(s: &str, field: &str) -> Result<(), ServerFnError> {
    if s.chars().count() > MAX_TEXT {
        return Err(ServerFnError::new(format!(
            "{field} too long (max {MAX_TEXT} characters)"
        )));
    }
    Ok(())
}

/// Comment / reply bodies — much roomier than short fields.
pub fn comment(s: &str) -> Result<(), ServerFnError> {
    let max = crate::models::COMMENT_MAX_CHARS;
    if s.chars().count() > max {
        return Err(ServerFnError::new(format!(
            "Comment too long (max {max} characters)"
        )));
    }
    Ok(())
}
