use dioxus::prelude::ServerFnError;

const MAX_TEXT: usize = 500;

pub fn text(s: &str, field: &str) -> Result<(), ServerFnError> {
    if s.len() > MAX_TEXT {
        return Err(ServerFnError::new(format!(
            "{field} too long (max {MAX_TEXT} chars)"
        )));
    }
    Ok(())
}
