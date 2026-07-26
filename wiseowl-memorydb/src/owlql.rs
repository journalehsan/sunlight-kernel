//! Minimal OwlQL subset — compiles into [`MemoryQuery`].
//!
//! Supported:
//! ```text
//! FIND observation, user_knowledge
//! MATCH TOKENS [101, 203]
//! USING TOKENIZER 2 VERSION 1
//! WHERE confidence >= 800 AND scope = USER
//! ORDER BY relevance DESC
//! LIMIT 20
//! ```
//!
//! Also: `FIND ALL WHERE source_id = 42 LIMIT 50`
//!
//! Not supported: joins, subqueries, arbitrary expressions, UDFs.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::SourceId;

use crate::error::DbError;
use crate::query::{MemoryQuery, QueryOrder, SourceQuery};
use crate::record::{KindMask, LongTermMemoryKind, MemoryScope};
use crate::tokens::{TokenMatchMode, TokenQuery};

/// Parse a minimal OwlQL string into a typed query.
pub fn parse_owlql(input: &str) -> Result<MemoryQuery, DbError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(DbError::OwlQlParse("empty"));
    }
    // Normalize whitespace for simple scanning.
    let upper = s.to_ascii_uppercase();
    if !upper.starts_with("FIND ") {
        return Err(DbError::OwlQlParse("expected FIND"));
    }

    let mut q = MemoryQuery::default();
    q.kinds = KindMask::empty();

    // Split roughly on keywords.
    let find_rest = &s[5..];
    let (kinds_part, after_find) = split_keyword(find_rest, &["MATCH", "USING", "WHERE", "ORDER", "LIMIT", "AFTER"])?;
    parse_kinds(kinds_part.trim(), &mut q)?;

    let mut rest = after_find;
    // MATCH TOKENS [...]
    if starts_ci(rest, "MATCH") {
        let body = rest["MATCH".len()..].trim_start();
        if !starts_ci(body, "TOKENS") {
            return Err(DbError::OwlQlParse("expected TOKENS"));
        }
        let body = body["TOKENS".len()..].trim_start();
        let (list, after) = parse_bracket_list(body)?;
        let tokens = parse_u64_list(&list)?;
        q.token_match = Some(TokenQuery {
            tokenizer_id: 0,
            tokenizer_version: 0,
            token_ids: tokens,
            mode: TokenMatchMode::Any,
        });
        rest = after;
    }

    if starts_ci(rest, "USING") {
        let body = rest["USING".len()..].trim_start();
        if !starts_ci(body, "TOKENIZER") {
            return Err(DbError::OwlQlParse("expected TOKENIZER"));
        }
        let body = body["TOKENIZER".len()..].trim_start();
        let (tid_s, body) = take_number(body)?;
        let body = body.trim_start();
        if !starts_ci(body, "VERSION") {
            return Err(DbError::OwlQlParse("expected VERSION"));
        }
        let body = body["VERSION".len()..].trim_start();
        let (ver_s, after) = take_number(body)?;
        let tid: u32 = tid_s
            .parse()
            .map_err(|_| DbError::OwlQlParse("tokenizer id"))?;
        let ver: u32 = ver_s
            .parse()
            .map_err(|_| DbError::OwlQlParse("tokenizer version"))?;
        if let Some(ref mut tq) = q.token_match {
            tq.tokenizer_id = tid;
            tq.tokenizer_version = ver;
        } else {
            return Err(DbError::OwlQlParse("USING without MATCH TOKENS"));
        }
        rest = after;
    }

    if starts_ci(rest, "WHERE") {
        let body = rest["WHERE".len()..].trim_start();
        let (where_part, after) =
            split_keyword(body, &["ORDER", "LIMIT", "AFTER"])?;
        parse_where(where_part, &mut q)?;
        rest = after;
    }

    if starts_ci(rest, "ORDER") {
        let body = rest["ORDER".len()..].trim_start();
        if !starts_ci(body, "BY") {
            return Err(DbError::OwlQlParse("expected BY"));
        }
        let body = body[2..].trim_start();
        let (order_part, after) = split_keyword(body, &["LIMIT", "AFTER"])?;
        q.order = parse_order(order_part.trim())?;
        rest = after;
    }

    if starts_ci(rest, "LIMIT") {
        let body = rest["LIMIT".len()..].trim_start();
        let (n, after) = take_number(body)?;
        q.limit = n.parse().map_err(|_| DbError::OwlQlParse("limit"))?;
        rest = after;
    }

    let rest = rest.trim();
    if !rest.is_empty() && rest != ";" {
        // Tolerate trailing semicolon only.
        let r = rest.trim_end_matches(';').trim();
        if !r.is_empty() {
            return Err(DbError::OwlQlParse("trailing junk"));
        }
    }

    if q.kinds.0 == 0 {
        q.kinds = KindMask::all();
    }
    Ok(q)
}

fn starts_ci(s: &str, kw: &str) -> bool {
    s.len() >= kw.len() && s[..kw.len()].eq_ignore_ascii_case(kw)
}

fn split_keyword<'a>(s: &'a str, kws: &[&str]) -> Result<(&'a str, &'a str), DbError> {
    let upper = s.to_ascii_uppercase();
    let mut best: Option<usize> = None;
    for kw in kws {
        if let Some(pos) = find_word(&upper, kw) {
            best = Some(best.map(|b| b.min(pos)).unwrap_or(pos));
        }
    }
    match best {
        Some(pos) => Ok((&s[..pos], s[pos..].trim_start())),
        None => Ok((s, "")),
    }
}

fn find_word(hay: &str, word: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = hay[start..].find(word) {
        let pos = start + rel;
        let before_ok = pos == 0 || hay.as_bytes()[pos - 1].is_ascii_whitespace();
        let end = pos + word.len();
        let after_ok = end >= hay.len() || hay.as_bytes()[end].is_ascii_whitespace();
        if before_ok && after_ok {
            return Some(pos);
        }
        start = pos + 1;
    }
    None
}

fn parse_kinds(part: &str, q: &mut MemoryQuery) -> Result<(), DbError> {
    let p = part.trim().trim_end_matches(';');
    if p.eq_ignore_ascii_case("ALL") || p.is_empty() {
        q.kinds = KindMask::all();
        return Ok(());
    }
    for piece in p.split(',') {
        let k = piece.trim().to_ascii_lowercase().replace('-', "_");
        let kind = match k.as_str() {
            "observation" => LongTermMemoryKind::Observation,
            "imported_record" | "imported" => LongTermMemoryKind::ImportedRecord,
            "user_knowledge" | "user_provided_knowledge" => {
                LongTermMemoryKind::UserProvidedKnowledge
            }
            "tool_verified_knowledge" | "tool_verified" => {
                LongTermMemoryKind::ToolVerifiedKnowledge
            }
            "remote_unverified_knowledge" | "remote" => {
                LongTermMemoryKind::RemoteUnverifiedKnowledge
            }
            "session_summary" => LongTermMemoryKind::SessionSummary,
            "preference" => LongTermMemoryKind::Preference,
            "procedure" => LongTermMemoryKind::Procedure,
            "diagnostic_history" | "diagnostic" => LongTermMemoryKind::DiagnosticHistory,
            _ => return Err(DbError::OwlQlParse("unknown kind")),
        };
        q.kinds = q.kinds.with(kind);
    }
    Ok(())
}

fn parse_bracket_list(s: &str) -> Result<(String, &str), DbError> {
    let s = s.trim_start();
    if !s.starts_with('[') {
        return Err(DbError::OwlQlParse("expected ["));
    }
    let end = s.find(']').ok_or(DbError::OwlQlParse("unclosed ["))?;
    let inner = s[1..end].to_string();
    Ok((inner, s[end + 1..].trim_start()))
}

fn parse_u64_list(s: &str) -> Result<Vec<u64>, DbError> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        out.push(p.parse().map_err(|_| DbError::OwlQlParse("token id"))?);
    }
    Ok(out)
}

fn take_number(s: &str) -> Result<(&str, &str), DbError> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        return Err(DbError::OwlQlParse("expected number"));
    }
    Ok((&s[..end], s[end..].trim_start()))
}

fn parse_where(part: &str, q: &mut MemoryQuery) -> Result<(), DbError> {
    // Split on AND (case insensitive).
    let upper = part.to_ascii_uppercase();
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut search = 0;
    while let Some(rel) = upper[search..].find(" AND ") {
        let pos = search + rel;
        pieces.push(part[start..pos].trim());
        start = pos + 5;
        search = start;
    }
    pieces.push(part[start..].trim());

    for piece in pieces {
        if piece.is_empty() {
            continue;
        }
        let p = piece.trim();
        if starts_ci(p, "CONFIDENCE") {
            let rest = p["CONFIDENCE".len()..].trim_start();
            if rest.starts_with(">=") {
                let n: u16 = rest[2..]
                    .trim()
                    .parse()
                    .map_err(|_| DbError::OwlQlParse("confidence"))?;
                q.min_confidence = Some(n);
            } else {
                return Err(DbError::OwlQlParse("confidence op"));
            }
        } else if starts_ci(p, "SCOPE") {
            let rest = p["SCOPE".len()..].trim_start();
            if !rest.starts_with('=') {
                return Err(DbError::OwlQlParse("scope op"));
            }
            let v = rest[1..].trim().to_ascii_lowercase();
            q.scope = Some(match v.as_str() {
                "user" => MemoryScope::User,
                "system" => MemoryScope::System,
                "session_derived" | "session" => MemoryScope::SessionDerived,
                "application" | "app" => MemoryScope::Application,
                _ => return Err(DbError::OwlQlParse("scope value")),
            });
        } else if starts_ci(p, "SOURCE_ID") {
            let rest = p["SOURCE_ID".len()..].trim_start();
            if !rest.starts_with('=') {
                return Err(DbError::OwlQlParse("source_id op"));
            }
            let n: u64 = rest[1..]
                .trim()
                .parse()
                .map_err(|_| DbError::OwlQlParse("source_id"))?;
            let sid = SourceId::from_raw(n).map_err(|_| DbError::OwlQlParse("source_id zero"))?;
            q.source = Some(SourceQuery {
                source_id: Some(sid),
                source_content_hash: None,
            });
        } else {
            return Err(DbError::OwlQlParse("unsupported where clause"));
        }
    }
    Ok(())
}

fn parse_order(part: &str) -> Result<QueryOrder, DbError> {
    let p = part.to_ascii_lowercase();
    if p.contains("relevance") {
        Ok(QueryOrder::TokenRelevanceDesc)
    } else if p.contains("confidence") {
        Ok(QueryOrder::ConfidenceDesc)
    } else if p.contains("importance") {
        Ok(QueryOrder::ImportanceDesc)
    } else if p.contains("recency") || p.contains("created") {
        Ok(QueryOrder::RecencyDesc)
    } else if p.contains("id") {
        Ok(QueryOrder::IdAsc)
    } else {
        Err(DbError::OwlQlParse("order"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_find_all_source() {
        let q = parse_owlql("FIND ALL WHERE source_id = 42 LIMIT 50").unwrap();
        assert_eq!(q.limit, 50);
        assert!(q.source.unwrap().source_id.unwrap().get() == 42);
    }

    #[test]
    fn parse_tokens() {
        let q = parse_owlql(
            "FIND observation MATCH TOKENS [101, 203] USING TOKENIZER 2 VERSION 1 WHERE confidence >= 800 AND scope = USER ORDER BY relevance DESC LIMIT 20",
        )
        .unwrap();
        assert!(q.kinds.contains(LongTermMemoryKind::Observation));
        let tq = q.token_match.unwrap();
        assert_eq!(tq.tokenizer_id, 2);
        assert_eq!(tq.tokenizer_version, 1);
        assert_eq!(tq.token_ids, vec![101, 203]);
        assert_eq!(q.min_confidence, Some(800));
        assert_eq!(q.scope, Some(MemoryScope::User));
        assert_eq!(q.limit, 20);
    }
}
