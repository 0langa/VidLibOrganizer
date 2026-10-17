//! Bounded, interpreted PCRE2-8. This is the only native regex boundary.
//!
//! Every native owner is non-null, uniquely owned and freed by its matching
//! PCRE2 allocator. Contexts and match data are local to a call; compiled code
//! is immutable after construction. No pointer escapes this module, no JIT
//! compilation is requested, and substitution explicitly sets NO_JIT.
use crate::{MatchScope, RenameError, RenameErrorCode, RuleIssue, RuleIssueCode};
use pcre2_sys::*;
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    ptr::{self, NonNull},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegexRule {
    pub pattern: String,
    pub replacement: String,
    pub scope: MatchScope,
    pub case_sensitive: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegexLimits {
    pub pattern_bytes: usize,
    pub match_limit: u32,
    pub depth_limit: u32,
    /// PCRE2's heap setter takes KiB, not bytes.
    pub heap_limit_kib: u32,
}
pub const REGEX_LIMITS: RegexLimits = RegexLimits {
    pattern_bytes: 16 * 1024,
    match_limit: 1_000_000,
    depth_limit: 1_000,
    heap_limit_kib: 8192,
};

macro_rules! owner {
    ($name:ident, $native:ty, $free:ident) => {
        #[derive(Debug)]
        struct $name(NonNull<$native>);
        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: constructed only from this allocator's non-null result;
                // this owner is never cloned and the allocation is freed once.
                unsafe { $free(self.0.as_ptr()) }
            }
        }
    };
}
owner!(Code, pcre2_code_8, pcre2_code_free_8);
owner!(
    CompileContext,
    pcre2_compile_context_8,
    pcre2_compile_context_free_8
);
owner!(
    MatchContext,
    pcre2_match_context_8,
    pcre2_match_context_free_8
);
owner!(MatchData, pcre2_match_data_8, pcre2_match_data_free_8);

#[derive(Debug)]
pub struct CompiledRegexRule {
    code: Code,
    replacement: String,
    scope: MatchScope,
}

fn native_issue(code: i32) -> RuleIssue {
    let mut buffer = [0u8; 256];
    // SAFETY: buffer is writable for its full supplied length; no retained pointer.
    let length = unsafe { pcre2_get_error_message_8(code, buffer.as_mut_ptr(), buffer.len()) };
    let detail = if length >= 0 {
        String::from_utf8_lossy(&buffer[..length as usize]).into_owned()
    } else {
        "unknown native error".into()
    };
    RuleIssue {
        code: if matches!(
            code,
            PCRE2_ERROR_MATCHLIMIT | PCRE2_ERROR_DEPTHLIMIT | PCRE2_ERROR_HEAPLIMIT
        ) {
            RuleIssueCode::RegexLimit
        } else {
            RuleIssueCode::RegexNative
        },
        native_code: Some(code),
        message: format!("PCRE2 {code}: {detail}"),
    }
}
fn allocation<T>(pointer: *mut T) -> Result<NonNull<T>, RuleIssue> {
    NonNull::new(pointer).ok_or_else(|| native_issue(PCRE2_ERROR_NOMEMORY))
}
fn checked(result: i32) -> Result<(), RuleIssue> {
    if result == 0 {
        Ok(())
    } else {
        Err(native_issue(result))
    }
}
fn compile_failure(issue: RuleIssue) -> RenameError {
    RenameError::new(RenameErrorCode::RegexNative, issue.message)
}

impl CompiledRegexRule {
    pub fn compile(rule: &RegexRule) -> Result<Self, RenameError> {
        if rule.pattern.len() > REGEX_LIMITS.pattern_bytes {
            return Err(crate::recipe::invalid("regex pattern exceeds 16 KiB"));
        }
        // SAFETY: null selects PCRE2's default allocator. The checked result is owned.
        let context = CompileContext(
            allocation(unsafe { pcre2_compile_context_create_8(ptr::null_mut()) })
                .map_err(compile_failure)?,
        );
        // SAFETY: live uniquely owned context; setter does not retain Rust pointers.
        checked(unsafe {
            pcre2_set_max_pattern_length_8(context.0.as_ptr(), REGEX_LIMITS.pattern_bytes)
        })
        .map_err(compile_failure)?;
        let mut error = 0;
        let mut offset = 0;
        let options = PCRE2_UTF
            | PCRE2_UCP
            | PCRE2_NEVER_BACKSLASH_C
            | if rule.case_sensitive {
                0
            } else {
                PCRE2_CASELESS
            };
        // SAFETY: pattern is valid UTF-8 and readable for exactly len bytes (no
        // terminating NUL needed). Outputs and context are valid for this call.
        // NEVER_BACKSLASH_C prevents captures splitting a Unicode scalar.
        let raw = unsafe {
            pcre2_compile_8(
                rule.pattern.as_ptr(),
                rule.pattern.len(),
                options,
                &mut error,
                &mut offset,
                context.0.as_ptr(),
            )
        };
        let code = Code(NonNull::new(raw).ok_or_else(|| {
            crate::recipe::invalid(format!(
                "{} at pattern byte {offset}",
                native_issue(error).message
            ))
        })?);
        #[cfg(test)]
        COMPILE_COUNT.with(|count| count.set(count.get() + 1));
        validate_replacement(&code, &rule.replacement)?;
        Ok(Self {
            code,
            replacement: rule.replacement.clone(),
            scope: rule.scope,
        })
    }

    /// Bounded interpreted match for selection; replacement text is never consulted.
    pub fn is_match(&self, input: &str) -> Result<bool, RuleIssue> {
        // SAFETY: default allocator; all contexts/data are owned throughout this call.
        let context = MatchContext(allocation(unsafe {
            pcre2_match_context_create_8(ptr::null_mut())
        })?);
        unsafe {
            checked(pcre2_set_match_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.match_limit,
            ))?;
            checked(pcre2_set_depth_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.depth_limit,
            ))?;
            checked(pcre2_set_heap_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.heap_limit_kib,
            ))?;
        }
        // SAFETY: compiled code is live and the checked allocation is uniquely owned.
        let data = MatchData(allocation(unsafe {
            pcre2_match_data_create_from_pattern_8(self.code.0.as_ptr(), ptr::null_mut())
        })?);
        // SAFETY: UTF-8 input has exactly the explicit readable length. PCRE2
        // retains no input pointers; code, context and data outlive the match.
        let result = unsafe {
            pcre2_match_8(
                self.code.0.as_ptr(),
                input.as_ptr(),
                input.len(),
                0,
                PCRE2_NO_JIT,
                data.0.as_ptr(),
                context.0.as_ptr(),
            )
        };
        match result {
            PCRE2_ERROR_NOMATCH => Ok(false),
            n if n >= 0 => Ok(true),
            code => Err(native_issue(code)),
        }
    }

    pub fn apply(&self, input: &str) -> Result<String, RuleIssue> {
        // SAFETY: default allocator; code lives throughout data creation and use.
        let context = MatchContext(allocation(unsafe {
            pcre2_match_context_create_8(ptr::null_mut())
        })?);
        // SAFETY: the context is live and uniquely owned. All setters return a
        // status, checked individually; these bounds apply to interpreter matches.
        unsafe {
            checked(pcre2_set_match_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.match_limit,
            ))?;
            checked(pcre2_set_depth_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.depth_limit,
            ))?;
            checked(pcre2_set_heap_limit_8(
                context.0.as_ptr(),
                REGEX_LIMITS.heap_limit_kib,
            ))?;
        }
        // SAFETY: live compiled code, default allocator, owned checked allocation.
        let data = MatchData(allocation(unsafe {
            pcre2_match_data_create_from_pattern_8(self.code.0.as_ptr(), ptr::null_mut())
        })?);
        let options = PCRE2_NO_JIT
            | PCRE2_SUBSTITUTE_UNSET_EMPTY
            | PCRE2_SUBSTITUTE_OVERFLOW_LENGTH
            | if self.scope == MatchScope::All {
                PCRE2_SUBSTITUTE_GLOBAL
            } else {
                0
            };
        let mut output = vec![0; input.len().saturating_add(1)];
        loop {
            let mut length = output.len();
            // SAFETY: input and replacement are readable UTF-8 for their explicit
            // lengths. Output is a separate writable allocation of length bytes.
            // Code, data and context outlive the call; PCRE2 retains no Rust buffers.
            // PCRE2 performs global empty-match retry/UTF advancement internally.
            let result = unsafe {
                pcre2_substitute_8(
                    self.code.0.as_ptr(),
                    input.as_ptr(),
                    input.len(),
                    0,
                    options,
                    data.0.as_ptr(),
                    context.0.as_ptr(),
                    self.replacement.as_ptr(),
                    self.replacement.len(),
                    output.as_mut_ptr(),
                    &mut length,
                )
            };
            if result == PCRE2_ERROR_NOMEMORY && length > output.len() && length != usize::MAX {
                // OVERFLOW_LENGTH returns the required capacity, including NUL.
                output
                    .try_reserve_exact(length - output.len())
                    .map_err(|_| native_issue(PCRE2_ERROR_NOMEMORY))?;
                output.resize(length, 0);
                continue;
            }
            if result < 0 {
                return Err(native_issue(result));
            }
            output.truncate(length);
            return String::from_utf8(output).map_err(|_| RuleIssue {
                code: RuleIssueCode::RegexNative,
                native_code: None,
                message: "PCRE2 returned invalid UTF-8".into(),
            });
        }
    }
}

/// Accept only $number, ${number}, ${name}, and $$. Backslashes stay literal.
/// Validate against the compiled pattern even when the subject never matches.
fn validate_replacement(code: &Code, replacement: &str) -> Result<(), RenameError> {
    let mut capture_count: u32 = 0;
    // SAFETY: CAPTURECOUNT writes exactly a u32 to this aligned output pointer.
    checked(unsafe {
        pcre2_pattern_info_8(
            code.0.as_ptr(),
            PCRE2_INFO_CAPTURECOUNT,
            (&mut capture_count as *mut u32).cast(),
        )
    })
    .map_err(compile_failure)?;
    let mut remaining = replacement;
    while let Some(index) = remaining.find('$') {
        remaining = &remaining[index + 1..];
        if let Some(tail) = remaining.strip_prefix('$') {
            remaining = tail;
            continue;
        }
        let reference;
        if let Some(braced) = remaining.strip_prefix('{') {
            let end = braced
                .find('}')
                .ok_or_else(|| crate::recipe::invalid("unclosed regex capture reference"))?;
            reference = &braced[..end];
            remaining = &braced[end + 1..];
        } else {
            let end = remaining.bytes().take_while(u8::is_ascii_digit).count();
            reference = &remaining[..end];
            remaining = &remaining[end..];
        }
        if reference.is_empty() {
            return Err(crate::recipe::invalid(
                "empty or malformed regex capture reference",
            ));
        }
        if reference.bytes().all(|b| b.is_ascii_digit()) {
            if reference
                .parse::<u32>()
                .ok()
                .is_none_or(|n| n > capture_count)
            {
                return Err(crate::recipe::invalid(
                    "unknown numeric regex capture reference",
                ));
            }
        } else {
            let name = CString::new(reference)
                .map_err(|_| crate::recipe::invalid("invalid regex capture name"))?;
            let mut first = ptr::null();
            let mut last = ptr::null();
            // SAFETY: live code and NUL-terminated name; both output slots are
            // valid pointer-sized storage. Returned pointers borrow code and are
            // never dereferenced or retained. Supplying both accepts duplicate
            // names; PCRE2 substitution chooses the first participating capture.
            let result = unsafe {
                pcre2_substring_nametable_scan_8(
                    code.0.as_ptr(),
                    name.as_ptr().cast(),
                    &mut first,
                    &mut last,
                )
            };
            if result < 0 {
                return Err(crate::recipe::invalid(
                    "unknown named regex capture reference",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
thread_local! { static COMPILE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use vidlib_inventory::*;

    #[test]
    fn evaluation_compiles_each_enabled_rule_once_for_all_rows() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.mkv", "aa.mkv", "aaa.mkv"] {
            std::fs::write(dir.path().join(name), b"media").unwrap();
        }
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let ids: Vec<_> = inventory
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect();
        let make_rule = |id: &str, enabled, pattern: &str, replacement: &str| Rule {
            id: id.into(),
            enabled,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Regex {
                rule: RegexRule {
                    pattern: pattern.into(),
                    replacement: replacement.into(),
                    scope: MatchScope::All,
                    case_sensitive: true,
                },
            },
        };
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: 1,
            selection: ids.clone(),
            selection_filter: None,
            numbering_order: ids.clone(),
            numbering_scope: Default::default(),
            rules: vec![
                make_rule("one", true, "a", "b"),
                make_rule("disabled", false, "(", "$99"),
                make_rule("two", true, "b", "c"),
            ],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let inputs = FrozenInputs {
            inventory_id: inventory.id,
            generation: inventory.generation,
            now_ns: 0,
            timezone: FrozenTimezone::Utc,
            primary_order: ids,
            numbering_scope: Default::default(),
            values: vec![],
        };
        COMPILE_COUNT.with(|count| count.set(0));
        let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
        assert_eq!(batch.rows.len(), 3);
        for row in batch.rows {
            assert_eq!(row.new_display_name, row.old_display_name.replace('a', "c"));
        }
        // Counts actual successful pcre2_compile_8 calls, per test thread.
        // A per-row compilation or validation+evaluation double compile fails.
        COMPILE_COUNT.with(|count| assert_eq!(count.get(), 2));
    }
}
