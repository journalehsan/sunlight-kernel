use super::*;
use network::{NetworkRule, ResourceType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    TooLarge,
    Limit,
    InvalidList,
    NoSupportedRules,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum RejectReason {
    Oversized,
    Syntax,
    Option,
    Selector,
    Domain,
    Regex,
    Transformation,
    Limit,
}
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub supported: usize,
    pub unsupported: usize,
    pub comments: usize,
    pub duplicates: usize,
    pub reasons: [usize; 8],
    /// Bounded line numbers/categories; never includes browsing URLs or content.
    pub samples: Vec<(usize, RejectReason)>,
}
impl Diagnostics {
    fn reject(&mut self, line: usize, reason: RejectReason) {
        self.unsupported += 1;
        self.reasons[reason as usize] += 1;
        if self.samples.len() < 32 {
            self.samples.push((line, reason));
        }
    }
}

pub fn compile(text: &str, selector_valid: fn(&str) -> bool) -> Result<CompiledList, CompileError> {
    compile_source(text, selector_valid, SourceText(None))
}
pub fn compile_static(
    text: &'static str,
    selector_valid: fn(&str) -> bool,
) -> Result<CompiledList, CompileError> {
    compile_source(text, selector_valid, SourceText(Some(text)))
}
#[derive(Clone, Copy)]
struct SourceText(Option<&'static str>);
impl SourceText {
    fn value(self, value: &str) -> Cow<'static, str> {
        if let Some(source) = self.0 {
            if let Some(start) = (value.as_ptr() as usize).checked_sub(source.as_ptr() as usize) {
                if let Some(end) = start.checked_add(value.len()) {
                    if let Some(slice) = source.get(start..end) {
                        return Cow::Borrowed(slice);
                    }
                }
            }
        }
        Cow::Owned(value.into())
    }
    fn normalized(self, original: &str, normalized: String) -> Cow<'static, str> {
        if original == normalized {
            self.value(original)
        } else {
            Cow::Owned(normalized)
        }
    }
}
fn compile_source(
    text: &str,
    selector_valid: fn(&str) -> bool,
    source: SourceText,
) -> Result<CompiledList, CompileError> {
    if text.len() > MAX_LIST_BYTES {
        return Err(CompileError::TooLarge);
    }
    // Conditional/include directives cannot be dropped as ordinary comments:
    // doing so would activate rules in a branch we do not understand.
    if text.lines().any(|line| line.trim_start().starts_with("!#")) {
        return Err(CompileError::InvalidList);
    }
    if !text
        .lines()
        .next()
        .is_some_and(|l| l.trim().starts_with("[Adblock Plus ") && l.trim().ends_with(']'))
    {
        return Err(CompileError::InvalidList);
    }
    let mut out = CompiledList {
        diagnostics: Diagnostics::default(),
        version: None,
        checksum: None,
        cosmetic: Vec::new(),
        network: Vec::new(),
        host_index: Vec::new(),
        token_index: Vec::new(),
        fallback: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    for (line_no, raw) in text.lines().enumerate().skip(1) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if raw.len() > MAX_LINE_BYTES {
            out.diagnostics.reject(line_no + 1, RejectReason::Oversized);
            continue;
        }
        if line.starts_with('!') {
            out.diagnostics.comments += 1;
            if let Some(v) = line.strip_prefix("! Version:") {
                out.version = Some(v.trim().into());
            }
            if let Some(v) = line.strip_prefix("! Checksum:") {
                out.checksum = Some(v.trim().into());
            }
            continue;
        }
        if out.network.len() + out.cosmetic.len() >= MAX_RULES {
            return Err(CompileError::Limit);
        }
        if !seen.insert(line) {
            out.diagnostics.duplicates += 1;
            continue;
        }
        let result = if let Some((domains, selector, exception)) = line
            .split_once("#@#")
            .map(|(d, s)| (d, s, true))
            .or_else(|| line.split_once("##").map(|(d, s)| (d, s, false)))
        {
            (|| {
                let domains = parse_domains(domains, ',', source)?;
                let selector = selector.trim();
                if selector.is_empty()
                    || selector.len() > 256
                    || selector.starts_with('+')
                    || selector.contains(['{', '}', ';', '\\'])
                    || !selector_valid(selector)
                {
                    return Err(RejectReason::Selector);
                }
                out.cosmetic.push(CosmeticRule {
                    domains,
                    selector: source.value(selector),
                    exception,
                });
                Ok(())
            })()
        } else {
            parse_network(line, source).and_then(|rule| {
                let i = out.network.len();
                if let Some(host) = &rule.host {
                    out.host_index.push((host.clone(), i));
                } else {
                    let literal = rule
                        .pattern
                        .split(['*', '^'])
                        .max_by_key(|s| s.len())
                        .unwrap_or("");
                    if literal.len() >= 4 {
                        out.token_index
                            .push((network::token(literal.as_bytes()), i));
                    } else {
                        if out.fallback.len() >= 256 {
                            return Err(RejectReason::Limit);
                        }
                        out.fallback.push(i);
                    }
                }
                out.network.push(rule);
                Ok(())
            })
        };
        match result {
            Ok(()) => out.diagnostics.supported += 1,
            Err(reason) => out.diagnostics.reject(line_no + 1, reason),
        }
    }
    if out.diagnostics.supported == 0 {
        return Err(CompileError::NoSupportedRules);
    }
    out.host_index.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    out.token_index.sort_unstable_by_key(|a| a.0);
    Ok(out)
}
fn parse_domains(
    input: &str,
    separator: char,
    source: SourceText,
) -> Result<Domains, RejectReason> {
    if input.is_empty() {
        return Ok(Domains::default());
    }
    let mut normalized = String::new();
    let mut has_inclusions = false;
    for (i, item) in input.split(separator).enumerate() {
        if i >= 64 {
            return Err(RejectReason::Limit);
        }
        let (exclude, host) = item.strip_prefix('~').map_or((false, item), |h| (true, h));
        let host = canonical_host(host).ok_or(RejectReason::Domain)?;
        if i > 0 {
            normalized.push(separator);
        }
        if exclude {
            normalized.push('~');
        } else {
            has_inclusions = true;
        }
        normalized.push_str(&host);
    }
    Ok(Domains {
        raw: source.normalized(input, normalized),
        separator,
        has_inclusions,
    })
}
fn parse_network(line: &str, source: SourceText) -> Result<NetworkRule, RejectReason> {
    let (exception, line) = line.strip_prefix("@@").map_or((false, line), |s| (true, s));
    if line.contains('#') || line.contains(['\\', '{', '}']) {
        return Err(RejectReason::Transformation);
    }
    let (pattern, options) = line
        .split_once('$')
        .map_or((line, None), |(p, o)| (p, Some(o)));
    if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 1 {
        return Err(RejectReason::Regex);
    }
    if pattern.is_empty()
        || pattern.len() > 512
        || !pattern.is_ascii()
        || pattern.bytes().any(|b| b.is_ascii_whitespace())
    {
        return Err(RejectReason::Syntax);
    }
    let mut rule = NetworkRule {
        pattern: Cow::Borrowed(""),
        host: None,
        domains: Domains::default(),
        exception,
        cosmetic_exception: 0,
        types: u16::MAX,
        party: None,
        start: false,
        end: false,
        match_case: false,
    };
    let mut include = 0u16;
    let mut exclude = 0u16;
    let mut seen = BTreeSet::new();
    if let Some(options) = options {
        if options.is_empty() {
            return Err(RejectReason::Option);
        }
        for option in options.split(',') {
            let (negated, option) = option
                .strip_prefix('~')
                .map_or((false, option), |s| (true, s));
            let key = option.split('=').next().unwrap_or("");
            if !seen.insert(key) {
                return Err(RejectReason::Option);
            }
            let kind = match option {
                "image" => Some(ResourceType::Image),
                "stylesheet" => Some(ResourceType::Stylesheet),
                "script" => Some(ResourceType::Script),
                "subdocument" => Some(ResourceType::Frame),
                "media" => Some(ResourceType::Media),
                "other" => Some(ResourceType::Other),
                "xmlhttprequest" => Some(ResourceType::XmlHttpRequest),
                "font" => Some(ResourceType::Font),
                _ => None,
            };
            if let Some(kind) = kind {
                if negated {
                    exclude |= kind.bit();
                } else {
                    include |= kind.bit();
                }
                continue;
            }
            match option {
                "third-party" => rule.party = Some(!negated),
                "match-case" if !negated => rule.match_case = true,
                "elemhide" if exception && !negated => rule.cosmetic_exception = 1,
                "generichide" if exception && !negated => {
                    // Disabling all element hiding subsumes disabling generic
                    // hiding, regardless of option order.
                    if rule.cosmetic_exception == 0 {
                        rule.cosmetic_exception = 2;
                    }
                }
                _ if option.starts_with("domain=") && !negated && option.len() > 7 => {
                    rule.domains = parse_domains(&option[7..], '|', source)?
                }
                _ => return Err(RejectReason::Option),
            }
        }
    }
    if rule.cosmetic_exception != 0 && (include != 0 || exclude != 0 || rule.party.is_some()) {
        return Err(RejectReason::Option);
    }
    rule.types = if include == 0 { u16::MAX } else { include };
    rule.types &= !exclude;
    let mut pattern = pattern;
    if let Some(rest) = pattern.strip_prefix("||") {
        let end = rest.find(['^', '/', '*', ':', '|']).unwrap_or(rest.len());
        rule.host = Some(source.normalized(
            &rest[..end],
            canonical_host(&rest[..end]).ok_or(RejectReason::Domain)?,
        ));
        if rest[end..].starts_with('*') {
            return Err(RejectReason::Domain);
        }
        pattern = rest;
    } else if let Some(rest) = pattern.strip_prefix('|') {
        rule.start = true;
        pattern = rest;
    }
    if let Some(rest) = pattern.strip_suffix('|') {
        rule.end = true;
        pattern = rest;
    }
    if pattern.is_empty() || pattern.contains('|') {
        return Err(RejectReason::Syntax);
    }
    rule.pattern = if let Some(host) = &rule.host {
        let original_host_end = pattern
            .find(['^', '/', '*', ':', '|'])
            .unwrap_or(pattern.len());
        if &pattern[..original_host_end] == host.as_ref() {
            source.value(pattern)
        } else {
            Cow::Owned(alloc::format!("{host}{}", &pattern[original_host_end..]))
        }
    } else {
        source.value(pattern)
    };
    Ok(rule)
}
