//! Shared generated-name budget, independent of native final-path limits.
use crate::{RuleIssue, RuleIssueCode};
pub const MAX_OUTPUT_BYTES: usize = 65_536;
pub(crate) fn limit() -> RuleIssue {
    RuleIssue {
        code: RuleIssueCode::OutputLimit,
        native_code: None,
        message: "generated intermediate name exceeds 65536 UTF-8 bytes".into(),
    }
}
pub(crate) fn size(length: usize) -> Result<(), RuleIssue> {
    if length > MAX_OUTPUT_BYTES {
        Err(limit())
    } else {
        Ok(())
    }
}
pub(crate) fn sum(lengths: impl IntoIterator<Item = usize>) -> Result<usize, RuleIssue> {
    let mut total = 0usize;
    for length in lengths {
        total = total.checked_add(length).ok_or_else(limit)?;
        size(total)?;
    }
    Ok(total)
}
pub(crate) fn append(output: &mut String, text: &str) -> Result<(), RuleIssue> {
    sum([output.len(), text.len()])?;
    output.push_str(text);
    Ok(())
}
pub(crate) fn push(output: &mut String, ch: char) -> Result<(), RuleIssue> {
    sum([output.len(), ch.len_utf8()])?;
    output.push(ch);
    Ok(())
}
pub(crate) fn concat(parts: &[&str]) -> Result<String, RuleIssue> {
    let length = sum(parts.iter().map(|p| p.len()))?;
    let mut output = String::with_capacity(length);
    for part in parts {
        output.push_str(part);
    }
    Ok(output)
}
// fmt::Write stops formatting before an oversized chunk is copied.
#[derive(Default)]
pub(crate) struct Writer {
    pub text: String,
    pub exceeded: bool,
}
impl std::fmt::Write for Writer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if append(&mut self.text, text).is_err() {
            self.exceeded = true;
            return Err(std::fmt::Error);
        }
        Ok(())
    }
}
